//! Safe typed helpers over the installed control-plane hook and event brokers.
//!
//! `publish_lifecycle_event` is deferred observation. `begin_lifecycle_hook`
//! uses the ordered, bounded request broker and must not be called from a
//! realtime capture, encode, network or input callback. Neither path grants
//! access to frame bytes, GPU handles or native interface tables.

use crate::{Command, ModuleResult, Response, RuntimeClient};
use amaterasu_contracts::hooks::{
    HookDecision, HookEvent, HookMode, HookReply, HookVersion, MAX_HOOK_EVENT_BYTES,
};
use amaterasu_contracts::valid_name;
use serde_json::Value;
use std::collections::BTreeSet;

const MAX_HOOK_PARTICIPANTS: usize = 32;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HookInfo {
    pub hook: String,
    pub owner: String,
    /// Installed policy order, independent of participant registration order.
    pub participants: Vec<String>,
    pub registered: Vec<String>,
    pub timeout_ms: u64,
}

/// One broker-owned invocation. Poll from the module tick or another bounded
/// control thread, never from a native media callback. Do not discard a pending
/// invocation while provider code might unload; unregister returns `busy` until
/// all references have settled or the installed timeout expires.
#[derive(Clone, Debug)]
pub struct HookInvocation {
    id: u64,
    event: HookEvent,
}

impl HookInvocation {
    pub fn id(&self) -> u64 {
        self.id
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParticipantDecision {
    pub participant: String,
    pub reply: HookReply,
}

impl RuntimeClient {
    /// Discover exactly one installed hook. Broker authorization limits this to
    /// the owner or a configured participant with a matching effective grant.
    pub fn query_lifecycle_hook(&self, hook: &str) -> ModuleResult<HookInfo> {
        match self.send(Command::QueryHook {
            hook: hook.to_owned(),
        })? {
            Response::HookInfo {
                hook,
                owner,
                participants,
                registered,
                timeout_ms,
            } => {
                let info = HookInfo {
                    hook,
                    owner,
                    participants,
                    registered,
                    timeout_ms,
                };
                validate_hook_info(&info)?;
                Ok(info)
            }
            response => Err(format!("hook discovery failed: {response:?}")),
        }
    }

    /// Register this module as one participant in an installed signed policy.
    /// Registration alone grants no access to other modules' buffers or APIs.
    pub fn register_lifecycle_hook(&self, hook: &str) -> ModuleResult<()> {
        self.require_success(Command::RegisterHook {
            hook: hook.to_owned(),
        })
    }

    /// Explicit unload barrier. A `busy` response means callbacks remain
    /// referenced; keep module code loaded and retry after the broker settles.
    pub fn unregister_lifecycle_hook(&self, hook: &str) -> ModuleResult<()> {
        self.require_success(Command::UnregisterHook {
            hook: hook.to_owned(),
        })
    }

    /// Publish metadata to an existing bounded topic. The installed publish
    /// grant and subscription capacity decide delivery/backpressure. This does
    /// not run ordered hook participants or apply modifications.
    pub fn publish_lifecycle_event(&self, topic: &str, event: &HookEvent) -> ModuleResult<()> {
        if event.mode != HookMode::Observe {
            return Err("deferred lifecycle publication requires observation mode".into());
        }
        let payload = encode_event(event)?;
        self.require_success(Command::Publish {
            topic: topic.to_owned(),
            payload,
        })
    }

    /// Begin one ordered control-plane hook. Proposals are never applied by the
    /// broker; the owner validates them and explicitly decides what to do.
    pub fn begin_lifecycle_hook(
        &self,
        hook: &str,
        event: &HookEvent,
    ) -> ModuleResult<HookInvocation> {
        let payload = encode_event(event)?;
        match self.send(Command::RunHook {
            hook: hook.to_owned(),
            payload,
        })? {
            Response::Accepted { id } if id != 0 => Ok(HookInvocation {
                id,
                event: event.clone(),
            }),
            response => Err(format!("hook invocation failed: {response:?}")),
        }
    }

    /// Poll once. `None` means pending. Validation fails closed on malformed,
    /// duplicate or mismatched replies; no participant can silently transform
    /// an observation into a cancellation proposal.
    pub fn poll_lifecycle_hook(
        &self,
        invocation: &HookInvocation,
    ) -> ModuleResult<Option<Vec<ParticipantDecision>>> {
        match self.send(Command::HookResult {
            invocation: invocation.id,
        })? {
            Response::Pending => Ok(None),
            Response::HookResult { proposals } => {
                if proposals.len() > MAX_HOOK_PARTICIPANTS {
                    return Err("hook result contains too many participants".into());
                }
                let mut seen = BTreeSet::new();
                let mut decisions = Vec::with_capacity(proposals.len());
                for proposal in proposals {
                    if !seen.insert(proposal.participant.clone()) {
                        return Err("hook result has a duplicate participant".into());
                    }
                    let reply: HookReply = serde_json::from_value(proposal.payload)
                        .map_err(|_| "hook participant returned an invalid reply")?;
                    reply.validate_for(&invocation.event)?;
                    decisions.push(ParticipantDecision {
                        participant: proposal.participant,
                        reply,
                    });
                }
                Ok(Some(decisions))
            }
            response => Err(format!("hook result failed: {response:?}")),
        }
    }

    /// Abandon an invocation owned by this module. This explicitly asks the
    /// broker to cancel and release its active request, then retire queued
    /// participant references. A completed/unknown invocation returns an error;
    /// callers should track whether they already received a terminal result.
    /// There is deliberately no network call in `HookInvocation::drop`.
    pub fn cancel_lifecycle_hook(&self, invocation: &HookInvocation) -> ModuleResult<()> {
        self.require_success(Command::CancelHook {
            invocation: invocation.id,
        })
    }
}

/// Decode one participant request. The owner supplies the request through the
/// normal `Poll` / `Complete` broker path; handlers own their returned reply.
pub fn decode_lifecycle_event(payload: Value) -> ModuleResult<HookEvent> {
    if serde_json::to_vec(&payload)
        .map_err(|error| error.to_string())?
        .len()
        > MAX_HOOK_EVENT_BYTES
    {
        return Err("lifecycle hook event is too large".into());
    }
    let event: HookEvent =
        serde_json::from_value(payload).map_err(|_| "lifecycle hook event has invalid fields")?;
    event.validate()?;
    Ok(event)
}

/// Produce a typed participant response. A proposal is returned to the owner,
/// which may reject it; the runtime broker does not mutate media/input state.
pub fn encode_lifecycle_reply(event: &HookEvent, decision: HookDecision) -> ModuleResult<Value> {
    event.validate()?;
    let reply = HookReply {
        version: HookVersion::CURRENT,
        event_id: event.event_id,
        decision,
    };
    reply.validate_for(event)?;
    serde_json::to_value(reply).map_err(|error| error.to_string())
}

fn encode_event(event: &HookEvent) -> ModuleResult<Value> {
    event.validate()?;
    let value = serde_json::to_value(event).map_err(|error| error.to_string())?;
    if serde_json::to_vec(&value)
        .map_err(|error| error.to_string())?
        .len()
        > MAX_HOOK_EVENT_BYTES
    {
        return Err("lifecycle hook event is too large".into());
    }
    Ok(value)
}

fn validate_hook_info(info: &HookInfo) -> ModuleResult<()> {
    if !valid_name(&info.hook)
        || !valid_name(&info.owner)
        || !(1..=30_000).contains(&info.timeout_ms)
        || info.participants.len() > MAX_HOOK_PARTICIPANTS
        || info.registered.len() > info.participants.len()
    {
        return Err("runtime returned an invalid hook descriptor".into());
    }
    let mut seen = BTreeSet::new();
    let mut registered = BTreeSet::new();
    if info
        .participants
        .iter()
        .any(|participant| !valid_name(participant) || !seen.insert(participant.as_str()))
        || info.registered.iter().any(|participant| {
            !seen.contains(participant.as_str()) || !registered.insert(participant.as_str())
        })
    {
        return Err("runtime returned an inconsistent hook descriptor".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FrameMemory, HookPoint, HookSubject, PixelFormat};
    use std::collections::VecDeque;
    use std::ffi::c_void;

    struct FakeEndpoint {
        commands: Vec<Value>,
        replies: VecDeque<Response>,
    }

    unsafe extern "C" fn invoke(
        context: *mut c_void,
        input: crate::abi::BorrowedBytes,
        sink: crate::abi::ReplySink,
    ) -> crate::abi::Status {
        // SAFETY: each test keeps this stack-owned endpoint alive for the call.
        let state = unsafe { &mut *context.cast::<FakeEndpoint>() };
        let bytes = unsafe { input.read() }.unwrap();
        state.commands.push(serde_json::from_slice(bytes).unwrap());
        let reply = state.replies.pop_front().unwrap();
        let bytes = serde_json::to_vec(&reply).unwrap();
        unsafe { sink.send(&bytes) };
        crate::abi::STATUS_OK
    }

    fn client(state: &mut FakeEndpoint) -> RuntimeClient {
        // SAFETY: the caller retains state through every synchronous SDK call.
        unsafe {
            RuntimeClient::from_endpoint(crate::abi::HostApi {
                header: crate::abi::AbiHeader::of::<crate::abi::HostApi>(),
                context: (state as *mut FakeEndpoint).cast(),
                invoke,
            })
        }
    }

    fn event() -> HookEvent {
        HookEvent {
            version: HookVersion::CURRENT,
            event_id: 8,
            point: HookPoint::BeforeFrame,
            mode: HookMode::Observe,
            subject: HookSubject::VideoFrame {
                width: 1280,
                height: 720,
                format: PixelFormat::Bgra,
                memory: FrameMemory::CpuBorrowed,
                timestamp_100ns: 1_000,
            },
        }
    }

    #[test]
    fn typed_observation_round_trips_without_a_buffer() {
        let value = encode_event(&event()).unwrap();
        assert!(value["subject"].get("bytes").is_none());
        assert_eq!(decode_lifecycle_event(value).unwrap(), event());
    }

    #[test]
    fn participant_cannot_cancel_an_observation() {
        assert!(encode_lifecycle_reply(&event(), HookDecision::Continue).is_ok());
        assert!(
            encode_lifecycle_reply(
                &event(),
                HookDecision::ProposeCancel {
                    reason: amaterasu_contracts::hooks::HookCancelReason::RejectedByExtension,
                },
            )
            .is_err()
        );
    }

    #[test]
    fn discovery_rejects_duplicate_and_unlisted_registration() {
        let mut info = HookInfo {
            hook: "video.before_frame/1".into(),
            owner: "amaterasu.video".into(),
            participants: vec!["example.one".into(), "example.two".into()],
            registered: vec!["example.one".into()],
            timeout_ms: 20,
        };
        assert!(validate_hook_info(&info).is_ok());
        info.participants.push("example.one".into());
        assert!(validate_hook_info(&info).is_err());
        info.participants.pop();
        info.registered.push("example.other".into());
        assert!(validate_hook_info(&info).is_err());
        info.registered.pop();
        info.registered.push("example.one".into());
        assert!(validate_hook_info(&info).is_err());
    }

    #[test]
    fn sdk_uses_the_existing_grant_checked_broker_commands() {
        let mut state = FakeEndpoint {
            commands: vec![],
            replies: VecDeque::from(vec![
                Response::HookInfo {
                    hook: "video.before_frame/1".into(),
                    owner: "amaterasu.video".into(),
                    participants: vec!["example.observer".into()],
                    registered: vec![],
                    timeout_ms: 20,
                },
                Response::Ok,
                Response::Ok,
                Response::Ok,
            ]),
        };
        let runtime = client(&mut state);
        let info = runtime
            .query_lifecycle_hook("video.before_frame/1")
            .unwrap();
        assert_eq!(info.participants, vec!["example.observer"]);
        runtime.register_lifecycle_hook(&info.hook).unwrap();
        runtime
            .publish_lifecycle_event("video.lifecycle/1", &event())
            .unwrap();
        runtime.unregister_lifecycle_hook(&info.hook).unwrap();
        assert_eq!(state.commands[0]["op"], "query_hook");
        assert_eq!(state.commands[1]["op"], "register_hook");
        assert_eq!(state.commands[2]["op"], "publish");
        assert_eq!(state.commands[2]["payload"]["point"], "before_frame");
        assert_eq!(state.commands[3]["op"], "unregister_hook");
    }

    #[test]
    fn deferred_publication_cannot_claim_to_propose_a_change() {
        let mut state = FakeEndpoint {
            commands: vec![],
            replies: VecDeque::new(),
        };
        let runtime = client(&mut state);
        let mut proposal = event();
        proposal.mode = HookMode::Propose;
        assert!(
            runtime
                .publish_lifecycle_event("video.lifecycle/1", &proposal)
                .is_err()
        );
        assert!(state.commands.is_empty());
    }

    #[test]
    fn invocation_rejects_mismatched_and_duplicate_participant_replies() {
        let invalid = serde_json::to_value(HookReply {
            version: HookVersion::CURRENT,
            event_id: 999,
            decision: HookDecision::Continue,
        })
        .unwrap();
        let valid = encode_lifecycle_reply(&event(), HookDecision::Continue).unwrap();
        let mut state = FakeEndpoint {
            commands: vec![],
            replies: VecDeque::from(vec![
                Response::Accepted { id: 3 },
                Response::Pending,
                Response::HookResult {
                    proposals: vec![amaterasu_contracts::Proposal {
                        participant: "example.observer".into(),
                        payload: invalid,
                    }],
                },
                Response::HookResult {
                    proposals: vec![
                        amaterasu_contracts::Proposal {
                            participant: "example.observer".into(),
                            payload: valid.clone(),
                        },
                        amaterasu_contracts::Proposal {
                            participant: "example.observer".into(),
                            payload: valid,
                        },
                    ],
                },
            ]),
        };
        let runtime = client(&mut state);
        let invocation = runtime
            .begin_lifecycle_hook("video.before_frame/1", &event())
            .unwrap();
        assert_eq!(invocation.id(), 3);
        assert!(runtime.poll_lifecycle_hook(&invocation).unwrap().is_none());
        assert!(runtime.poll_lifecycle_hook(&invocation).is_err());
        assert!(runtime.poll_lifecycle_hook(&invocation).is_err());
        assert_eq!(state.commands[0]["op"], "run_hook");
        assert_eq!(state.commands[1]["op"], "hook_result");
    }

    #[test]
    fn abandoned_invocation_requires_an_explicit_owner_cancel() {
        let mut state = FakeEndpoint {
            commands: vec![],
            replies: VecDeque::from(vec![Response::Accepted { id: 41 }, Response::Ok]),
        };
        let runtime = client(&mut state);
        let invocation = runtime
            .begin_lifecycle_hook("video.before_frame/1", &event())
            .unwrap();
        assert_eq!(state.commands.len(), 1);
        runtime.cancel_lifecycle_hook(&invocation).unwrap();
        assert_eq!(state.commands[1]["op"], "cancel_hook");
        assert_eq!(state.commands[1]["invocation"], 41);
    }
}
