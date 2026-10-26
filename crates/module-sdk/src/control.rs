//! Synchronous client for one host-created private plugin control pipe.
//!
//! This client never discovers a broker, authenticates an administrator,
//! approves a request, or retries a write. A failed write/read/parse leaves the
//! outcome uncertain and permanently retires this client: a late response must
//! never be mistaken for the next operation's response. The host owns timeouts;
//! embedding applications must not run blocking reads on a scheduler thread.
pub use amaterasu_contracts::extensions::control::{
    Capability, ControlReply, ControlRequest, ElevationOutcome, LaunchSpec,
    MAX_CONTROL_MESSAGE_BYTES, PermissionState,
};
use std::fmt;
use std::io::{BufRead, Read, Write};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlError {
    InvalidRequest(String),
    HostRejected(ControlReply),
    /// The operation may have happened. Do not replay it automatically.
    OutcomeUncertain(String),
    /// A previous uncertain operation retired this channel.
    Retired,
}

impl fmt::Display for ControlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(reason) => write!(formatter, "invalid control request: {reason}"),
            Self::HostRejected(reply) => {
                write!(formatter, "control request rejected: {}", reply.code)
            }
            Self::OutcomeUncertain(reason) => {
                write!(formatter, "control outcome uncertain: {reason}")
            }
            Self::Retired => formatter.write_str("control channel retired after uncertain outcome"),
        }
    }
}

impl std::error::Error for ControlError {}

pub struct ControlClient<R, W> {
    reader: R,
    writer: W,
    retired: bool,
}

impl<R: BufRead, W: Write> ControlClient<R, W> {
    /// Use only handles supplied by the host for this plugin's private channel.
    /// In a control executable these are normally locked stdin and stdout;
    /// keep diagnostic logging off that stdout. There is one outstanding call.
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            retired: false,
        }
    }

    pub fn is_retired(&self) -> bool {
        self.retired
    }

    pub fn request_permission(
        &mut self,
        capability: Capability,
        reason: String,
    ) -> Result<ControlReply, ControlError> {
        self.exchange(ControlRequest::Request { capability, reason })
    }

    pub fn permission_status(&mut self, request_id: String) -> Result<ControlReply, ControlError> {
        self.exchange(ControlRequest::Status { request_id })
    }

    pub fn invoke(
        &mut self,
        request_id: String,
        capability: Capability,
    ) -> Result<ControlReply, ControlError> {
        self.exchange(ControlRequest::Invoke {
            request_id,
            capability,
        })
    }

    pub fn cancel_permission(&mut self, request_id: String) -> Result<ControlReply, ControlError> {
        self.exchange(ControlRequest::Cancel { request_id })
    }

    pub fn request_elevation(
        &mut self,
        launch: LaunchSpec,
        reason: String,
    ) -> Result<ControlReply, ControlError> {
        self.exchange(ControlRequest::RequestElevation { launch, reason })
    }

    pub fn elevation_status(&mut self, request_id: String) -> Result<ControlReply, ControlError> {
        self.exchange(ControlRequest::ElevationStatus { request_id })
    }

    pub fn launch_elevated(&mut self, request_id: String) -> Result<ControlReply, ControlError> {
        self.exchange(ControlRequest::LaunchElevated { request_id })
    }

    pub fn cancel_elevation(&mut self, request_id: String) -> Result<ControlReply, ControlError> {
        self.exchange(ControlRequest::CancelElevation { request_id })
    }

    /// Exactly one write and one reply. An error after transmission begins
    /// poisons the channel even for reads, preventing reply desynchronization.
    pub fn exchange(&mut self, request: ControlRequest) -> Result<ControlReply, ControlError> {
        if self.retired {
            return Err(ControlError::Retired);
        }
        request
            .validate()
            .map_err(|reason| ControlError::InvalidRequest(reason.into()))?;
        let mut bytes = serde_json::to_vec(&request)
            .map_err(|error| ControlError::InvalidRequest(error.to_string()))?;
        bytes.push(b'\n');
        if bytes.len() > MAX_CONTROL_MESSAGE_BYTES {
            return Err(ControlError::InvalidRequest(
                "control message exceeds byte limit".into(),
            ));
        }
        self.retired = true;
        self.writer
            .write_all(&bytes)
            .and_then(|_| self.writer.flush())
            .map_err(|error| ControlError::OutcomeUncertain(error.to_string()))?;
        let mut reply_bytes = Vec::new();
        self.reader
            .by_ref()
            .take(MAX_CONTROL_MESSAGE_BYTES as u64 + 1)
            .read_until(b'\n', &mut reply_bytes)
            .map_err(|error| ControlError::OutcomeUncertain(error.to_string()))?;
        if reply_bytes.len() > MAX_CONTROL_MESSAGE_BYTES || reply_bytes.last() != Some(&b'\n') {
            return Err(ControlError::OutcomeUncertain(
                "unterminated or oversized control reply".into(),
            ));
        }
        let reply: ControlReply = serde_json::from_slice(&reply_bytes)
            .map_err(|error| ControlError::OutcomeUncertain(error.to_string()))?;
        reply
            .validate_for(&request)
            .map_err(|reason| ControlError::OutcomeUncertain(reason.into()))?;
        self.retired = false;
        if reply.ok {
            Ok(reply)
        } else {
            Err(ControlError::HostRejected(reply))
        }
    }

    /// Recover owned handles for explicit teardown or test inspection. A retired
    /// stream must not be reused for further control calls.
    pub fn into_parts(self) -> (R, W) {
        (self.reader, self.writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const ID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn capability() -> Capability {
        Capability {
            action: "demo.echo".into(),
            resource: "fixture".into(),
        }
    }

    #[test]
    fn typed_requests_preserve_states_and_cancellation_encoding() {
        let replies = format!(
            "{{\"ok\":true,\"code\":\"ok\",\"request_id\":\"{ID}\"}}\n\
            {{\"ok\":true,\"code\":\"ok\",\"request_id\":\"{ID}\",\"state\":\"pending\"}}\n\
            {{\"ok\":true,\"code\":\"ok\",\"request_id\":\"{ID}\",\"state\":\"revoked\"}}\n"
        );
        let mut client = ControlClient::new(Cursor::new(replies), Vec::new());
        assert_eq!(
            client
                .request_permission(capability(), "test".into())
                .unwrap()
                .request_id
                .as_deref(),
            Some(ID)
        );
        assert_eq!(
            client.permission_status(ID.into()).unwrap().state,
            Some(PermissionState::Pending)
        );
        assert_eq!(
            client.cancel_permission(ID.into()).unwrap().state,
            Some(PermissionState::Revoked)
        );
        let (_, written) = client.into_parts();
        let requests: Vec<ControlRequest> = String::from_utf8(written)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert!(matches!(requests[0], ControlRequest::Request { .. }));
        assert!(matches!(requests[1], ControlRequest::Status { .. }));
        assert!(matches!(requests[2], ControlRequest::Cancel { .. }));
    }

    #[test]
    fn typed_windows_launch_uses_the_shared_path_contract() {
        let reply = format!(
            "{{\"ok\":true,\"code\":\"ok\",\"request_id\":\"{ID}\",\"state\":\"pending\",\"attempted\":false}}\n"
        );
        let mut client = ControlClient::new(Cursor::new(reply), Vec::new());
        let cwd = r"C:\ProgramData\Amaterasu\ElevatedCwd";
        assert_eq!(
            client
                .request_elevation(
                    LaunchSpec {
                        arguments: vec!["--synthetic".into()],
                        environment: Default::default(),
                        cwd: cwd.into(),
                    },
                    "exact verified plugin launch".into(),
                )
                .unwrap()
                .request_id
                .as_deref(),
            Some(ID),
        );
        let (_, bytes) = client.into_parts();
        let sent: ControlRequest =
            serde_json::from_slice(bytes.strip_suffix(b"\n").unwrap()).unwrap();
        assert!(
            matches!(sent, ControlRequest::RequestElevation { launch, .. } if launch.cwd == cwd)
        );
    }

    #[test]
    fn eof_mismatch_and_oversize_retire_without_replaying() {
        for reply in [
            Vec::new(),
            vec![b'x'; MAX_CONTROL_MESSAGE_BYTES + 1],
            format!(
                "{{\"ok\":true,\"code\":\"ok\",\"request_id\":\"{}\",\"state\":\"pending\"}}\n",
                "b".repeat(32)
            )
            .into_bytes(),
        ] {
            let mut client = ControlClient::new(Cursor::new(reply), Vec::new());
            assert!(matches!(
                client.permission_status(ID.into()),
                Err(ControlError::OutcomeUncertain(_))
            ));
            assert!(client.is_retired());
            assert_eq!(
                client.permission_status(ID.into()),
                Err(ControlError::Retired)
            );
            let (_, written) = client.into_parts();
            assert_eq!(written.iter().filter(|byte| **byte == b'\n').count(), 1);
        }
    }

    #[test]
    fn local_validation_and_host_denial_do_not_desynchronize() {
        let mut client = ControlClient::new(
            Cursor::new(b"{\"ok\":false,\"code\":\"permission_denied\"}\n"),
            Vec::new(),
        );
        assert!(matches!(
            client.permission_status("bad".into()),
            Err(ControlError::InvalidRequest(_))
        ));
        assert!(!client.is_retired());
        assert!(matches!(
            client.permission_status(ID.into()),
            Err(ControlError::HostRejected(_))
        ));
        assert!(!client.is_retired());
        let (_, written) = client.into_parts();
        assert_eq!(written.iter().filter(|byte| **byte == b'\n').count(), 1);
    }

    #[test]
    fn partial_write_is_uncertain_and_is_never_retried() {
        struct FailedWriter;
        impl Write for FailedWriter {
            fn write(&mut self, _bytes: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("injected partial transport loss"))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut client = ControlClient::new(Cursor::new(Vec::<u8>::new()), FailedWriter);
        assert!(matches!(
            client.request_permission(capability(), "test".into()),
            Err(ControlError::OutcomeUncertain(_))
        ));
        assert_eq!(
            client.request_permission(capability(), "test".into()),
            Err(ControlError::Retired)
        );
    }
}
