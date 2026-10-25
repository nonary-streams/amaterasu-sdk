//! Typed control-plane lifecycle hook messages.
//!
//! These values travel through the existing bounded JSON hook broker. They carry
//! observations only: no frame bytes, GPU handles, borrowed pointers or native
//! callbacks cross this interface. A proposal is returned to the hook owner; the
//! broker never applies it to media or input on the participant's behalf.

use serde::{Deserialize, Serialize};

pub const HOOK_CONTRACT_MAJOR: u16 = 1;
pub const HOOK_CONTRACT_MINOR: u16 = 0;
pub const MAX_HOOK_EVENT_BYTES: usize = 8 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookVersion {
    pub major: u16,
    pub minor: u16,
}

impl HookVersion {
    pub const CURRENT: Self = Self {
        major: HOOK_CONTRACT_MAJOR,
        minor: HOOK_CONTRACT_MINOR,
    };

    pub fn is_supported(self) -> bool {
        self.major == HOOK_CONTRACT_MAJOR && self.minor == HOOK_CONTRACT_MINOR
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Names when a module recorded a lifecycle fact. On the deferred EventBus path,
/// even a `Before*` fact is delivered later and cannot intercept that stage.
pub enum HookPoint {
    BeforeCapture,
    AfterCapture,
    BeforeFrame,
    AfterFrame,
    BeforeEncode,
    AfterEncode,
    BeforePacket,
    AfterPacket,
    BeforeNetwork,
    AfterNetwork,
    BeforeInput,
    AfterInput,
    BeforeAudio,
    AfterAudio,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookMode {
    /// Retrospective metadata notification. Participants may acknowledge it but
    /// cannot intercept, veto or change the named stage.
    Observe,
    /// Participants may propose cancellation; the owner alone makes that choice.
    Propose,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameMemory {
    CpuOwned,
    CpuBorrowed,
    GpuOpaque,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    Bgra,
    Nv12,
    P010,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
    Unknown,
}

/// Bounded facts about a lifecycle point. Memory/storage variants are labels,
/// not an invitation to retain a provider's buffer or open its GPU resource.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookSubject {
    Capture {
        width: u32,
        height: u32,
        memory: FrameMemory,
    },
    VideoFrame {
        width: u32,
        height: u32,
        format: PixelFormat,
        memory: FrameMemory,
        timestamp_100ns: u64,
    },
    EncodedPacket {
        codec: VideoCodec,
        bytes: u32,
        timestamp_100ns: u64,
        keyframe: bool,
    },
    Network {
        bytes: u32,
        encrypted: bool,
    },
    Input {
        events: u16,
    },
    Audio {
        frames: u32,
        channels: u8,
        sample_rate_hz: u32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookEvent {
    pub version: HookVersion,
    /// Correlation ID unique within the producer epoch, possibly shared across
    /// topics. Gaps are allowed and do not prove a dropped observation.
    /// EventBus publisher sequence/resync covers only accepted publishes;
    /// a producer may drop locally before calling the broker.
    pub event_id: u64,
    pub point: HookPoint,
    pub mode: HookMode,
    pub subject: HookSubject,
}

impl HookEvent {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !self.version.is_supported() {
            return Err("unsupported hook contract version");
        }
        if self.event_id == 0 {
            return Err("hook event ID must be nonzero");
        }
        let kind_matches = matches!(
            (&self.point, &self.subject),
            (
                HookPoint::BeforeCapture | HookPoint::AfterCapture,
                HookSubject::Capture { .. }
            ) | (
                HookPoint::BeforeFrame | HookPoint::AfterFrame | HookPoint::BeforeEncode,
                HookSubject::VideoFrame { .. }
            ) | (
                HookPoint::AfterEncode | HookPoint::BeforePacket | HookPoint::AfterPacket,
                HookSubject::EncodedPacket { .. }
            ) | (
                HookPoint::BeforeNetwork | HookPoint::AfterNetwork,
                HookSubject::Network { .. }
            ) | (
                HookPoint::BeforeInput | HookPoint::AfterInput,
                HookSubject::Input { .. }
            ) | (
                HookPoint::BeforeAudio | HookPoint::AfterAudio,
                HookSubject::Audio { .. }
            )
        );
        if !kind_matches {
            return Err("hook subject does not match lifecycle point");
        }
        match &self.subject {
            HookSubject::Capture { width, height, .. }
            | HookSubject::VideoFrame { width, height, .. }
                if *width == 0 || *height == 0 || *width > 16_384 || *height > 16_384 =>
            {
                Err("hook frame dimensions are invalid")
            }
            HookSubject::Audio {
                frames,
                channels,
                sample_rate_hz,
            } if *frames == 0
                || *frames > 1_048_576
                || *channels == 0
                || *channels > 32
                || !(8_000..=384_000).contains(sample_rate_hz) =>
            {
                Err("hook audio description is invalid")
            }
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookCancelReason {
    RejectedByExtension,
    UnsupportedContent,
    ResourceUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HookDecision {
    Continue,
    ProposeCancel { reason: HookCancelReason },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HookReply {
    pub version: HookVersion,
    pub event_id: u64,
    pub decision: HookDecision,
}

impl HookReply {
    pub fn validate_for(&self, event: &HookEvent) -> Result<(), &'static str> {
        if self.version != event.version || self.event_id != event.event_id {
            return Err("hook reply does not match its event");
        }
        if event.mode == HookMode::Observe && !matches!(self.decision, HookDecision::Continue) {
            return Err("observation hook cannot propose cancellation");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_event() -> HookEvent {
        HookEvent {
            version: HookVersion::CURRENT,
            event_id: 7,
            point: HookPoint::BeforeEncode,
            mode: HookMode::Observe,
            subject: HookSubject::VideoFrame {
                width: 1920,
                height: 1080,
                format: PixelFormat::Nv12,
                memory: FrameMemory::GpuOpaque,
                timestamp_100ns: 10_000,
            },
        }
    }

    #[test]
    fn version_and_subject_must_match_the_point() {
        let mut event = frame_event();
        assert!(event.validate().is_ok());
        event.version.major += 1;
        assert!(event.validate().is_err());
        event.version = HookVersion::CURRENT;
        event.point = HookPoint::BeforeNetwork;
        assert!(event.validate().is_err());
    }

    #[test]
    fn observation_rejects_changes_and_mismatched_replies() {
        let event = frame_event();
        let mut reply = HookReply {
            version: HookVersion::CURRENT,
            event_id: event.event_id,
            decision: HookDecision::Continue,
        };
        assert!(reply.validate_for(&event).is_ok());
        reply.decision = HookDecision::ProposeCancel {
            reason: HookCancelReason::RejectedByExtension,
        };
        assert!(reply.validate_for(&event).is_err());
        reply.decision = HookDecision::Continue;
        reply.event_id += 1;
        assert!(reply.validate_for(&event).is_err());
    }

    #[test]
    fn metadata_wire_has_no_buffer_or_handle_fields() {
        let value = serde_json::to_value(frame_event()).unwrap();
        assert_eq!(value["subject"]["memory"], "gpu_opaque");
        assert!(value["subject"].get("pointer").is_none());
        assert!(value["subject"].get("handle").is_none());
        assert!(
            serde_json::from_value::<HookEvent>(serde_json::json!({
                "version": { "major": 1, "minor": 0 },
                "event_id": 1,
                "point": "before_encode",
                "mode": "observe",
                "subject": {
                    "kind": "video_frame", "width": 10, "height": 10,
                    "format": "nv12", "memory": "cpu_owned", "timestamp_100ns": 0,
                    "handle": 3
                }
            }))
            .is_err()
        );
    }
}
