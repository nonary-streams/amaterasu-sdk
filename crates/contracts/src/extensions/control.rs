//! Plugin-side messages for a host-created private control channel.
//!
//! The host supplies the authenticated plugin identity. These messages cannot
//! select another principal, approve a request, or confer OS administrator rights.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Includes the final newline. Both endpoints must reject larger messages.
pub const MAX_CONTROL_MESSAGE_BYTES: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capability {
    pub action: String,
    pub resource: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaunchSpec {
    /// Direct argv entries for the verified plugin itself, never a shell command.
    pub arguments: Vec<String>,
    /// The initial protocol requires this map to be empty.
    pub environment: BTreeMap<String, String>,
    /// The initial protocol accepts only `/`.
    pub cwd: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Pending,
    Approved,
    Denied,
    Revoked,
    Expired,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ElevationOutcome {
    AwaitingOsConsent,
    LaunchAccepted,
    LaunchUnavailable,
    LaunchUnconfirmed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlRequest {
    Request {
        capability: Capability,
        reason: String,
    },
    Status {
        request_id: String,
    },
    Invoke {
        request_id: String,
        capability: Capability,
    },
    /// Retire this child's request. Cancellation cannot undo a completed action.
    Cancel {
        request_id: String,
    },
    RequestElevation {
        launch: LaunchSpec,
        reason: String,
    },
    ElevationStatus {
        request_id: String,
    },
    LaunchElevated {
        request_id: String,
    },
    /// Retire this child's elevation request. Already launched code is not undone.
    CancelElevation {
        request_id: String,
    },
}

impl ControlRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Self::Request { capability, reason } => {
                validate_capability(capability)?;
                validate_reason(reason)
            }
            Self::RequestElevation { launch, reason } => {
                validate_reason(reason)?;
                if launch.cwd != "/"
                    || !launch.environment.is_empty()
                    || launch.arguments.len() > 16
                    || launch
                        .arguments
                        .iter()
                        .any(|value| value.len() > 256 || !review_text_safe(value))
                {
                    return Err("launch specification is outside the initial control contract");
                }
                Ok(())
            }
            Self::Invoke {
                request_id,
                capability,
            } => {
                validate_request_id(request_id)?;
                validate_capability(capability)
            }
            Self::Status { request_id }
            | Self::Cancel { request_id }
            | Self::ElevationStatus { request_id }
            | Self::LaunchElevated { request_id }
            | Self::CancelElevation { request_id } => validate_request_id(request_id),
        }
    }

    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Request { .. } | Self::RequestElevation { .. } => None,
            Self::Status { request_id }
            | Self::Invoke { request_id, .. }
            | Self::Cancel { request_id }
            | Self::ElevationStatus { request_id }
            | Self::LaunchElevated { request_id }
            | Self::CancelElevation { request_id } => Some(request_id),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlReply {
    pub ok: bool,
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<PermissionState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attempted: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elevation_outcome: Option<ElevationOutcome>,
}

impl ControlReply {
    /// Check framing-independent invariants and correlation before trusting a reply.
    pub fn validate_for(&self, request: &ControlRequest) -> Result<(), &'static str> {
        if self.code.is_empty()
            || self.code.len() > 128
            || !self
                .code
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte == b'_')
        {
            return Err("invalid control reply code");
        }
        if !self.ok {
            if self.code == "ok"
                || self.request_id.is_some()
                || self.state.is_some()
                || self.attempted.is_some()
                || self.elevation_outcome.is_some()
            {
                return Err("invalid control failure reply");
            }
            return Ok(());
        }
        if self.code != "ok" {
            return Err("inconsistent control success reply");
        }
        let id = self
            .request_id
            .as_deref()
            .ok_or("missing control request ID")?;
        validate_request_id(id)?;
        if request.request_id().is_some_and(|expected| expected != id) {
            return Err("control reply belongs to another request");
        }
        match request {
            ControlRequest::Cancel { .. } | ControlRequest::CancelElevation { .. }
                if self.state != Some(PermissionState::Revoked) =>
            {
                Err("cancellation did not retire the request")
            }
            ControlRequest::CancelElevation { .. } if self.attempted != Some(false) => {
                Err("cancellation cannot undo an attempted launch")
            }
            ControlRequest::Status { .. } if self.state.is_none() => {
                Err("missing permission state")
            }
            ControlRequest::RequestElevation { .. } | ControlRequest::ElevationStatus { .. }
                if self.state.is_none() || self.attempted.is_none() =>
            {
                Err("missing elevation state")
            }
            ControlRequest::LaunchElevated { .. }
                if self.state != Some(PermissionState::Approved)
                    || self.attempted != Some(true)
                    || self.elevation_outcome.is_none() =>
            {
                Err("missing launch attempt state")
            }
            _ => Ok(()),
        }
    }
}

fn validate_capability(capability: &Capability) -> Result<(), &'static str> {
    if capability.action.is_empty()
        || capability.action.len() > 96
        || !capability
            .action
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || matches!(byte, b'.' | b'_' | b'-'))
        || capability.resource.is_empty()
        || capability.resource.len() > 96
        || !capability
            .resource
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err("invalid capability");
    }
    Ok(())
}

fn validate_reason(reason: &str) -> Result<(), &'static str> {
    if reason.is_empty() || reason.len() > 256 || !review_text_safe(reason) {
        return Err("invalid permission reason");
    }
    Ok(())
}

fn validate_request_id(id: &str) -> Result<(), &'static str> {
    if id.len() != 32
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("invalid permission request ID");
    }
    Ok(())
}

fn review_text_safe(value: &str) -> bool {
    !value.chars().any(|character| {
        character.is_control()
            || matches!(character, '\u{061c}' | '\u{200e}' | '\u{200f}'
            | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requester_contract_cannot_encode_administrator_authority() {
        for raw in [
            r#"{"op":"approve","request_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#,
            r#"{"op":"request","capability":{"action":"demo.echo","resource":"fixture"},"reason":"why","principal":"admin"}"#,
        ] {
            assert!(serde_json::from_str::<ControlRequest>(raw).is_err());
        }
    }

    #[test]
    fn requests_bound_review_text_spec_and_identifiers() {
        let request = ControlRequest::RequestElevation {
            launch: LaunchSpec {
                arguments: vec!["--synthetic".into()],
                environment: BTreeMap::new(),
                cwd: "/".into(),
            },
            reason: "Synthetic test".into(),
        };
        assert!(request.validate().is_ok());
        let mut changed = request.clone();
        if let ControlRequest::RequestElevation { reason, .. } = &mut changed {
            reason.push('\u{202e}');
        }
        assert!(changed.validate().is_err());
        if let ControlRequest::RequestElevation { launch, .. } = &mut changed {
            launch
                .environment
                .insert("LD_PRELOAD".into(), "payload".into());
        }
        assert!(changed.validate().is_err());
        assert!(
            ControlRequest::Cancel {
                request_id: "other".into()
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn cancellation_reply_must_retire_an_unattempted_request() {
        let id = "a".repeat(32);
        let request = ControlRequest::CancelElevation {
            request_id: id.clone(),
        };
        let mut reply = ControlReply {
            ok: true,
            code: "ok".into(),
            request_id: Some(id),
            state: Some(PermissionState::Approved),
            attempted: Some(false),
            elevation_outcome: None,
        };
        assert!(reply.validate_for(&request).is_err());
        reply.state = Some(PermissionState::Revoked);
        assert!(reply.validate_for(&request).is_ok());
        reply.attempted = Some(true);
        assert!(reply.validate_for(&request).is_err());
    }
}
