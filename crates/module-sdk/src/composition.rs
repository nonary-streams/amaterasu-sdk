//! Host-only composition for independently owned extension pipelines.
//!
//! This utility routes *owned* packets from up to four pipelines to a caller's
//! demultiplexer. It neither invents a Moonlight multistream wire format nor
//! grants provider access. Each pipeline still needs its own authorized native
//! bindings, control operations and durable cleanup identity. No GPU handle or
//! borrowed frame crosses this API.

use std::collections::BTreeMap;

pub const MAX_COMPOSED_STREAMS: usize = 4;
pub const MAX_COMPOSED_PACKET_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct StreamId(u32);

impl StreamId {
    pub fn new(value: u32) -> Result<Self, CompositionError> {
        (value != 0)
            .then_some(Self(value))
            .ok_or(CompositionError::InvalidIdentity)
    }

    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Opaque host-side routing metadata. These names are *not* capabilities;
/// actual service binding must be independently checked by the runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamConfig {
    pub id: StreamId,
    pub capture_service: String,
    pub encoder_service: String,
    pub output_route: String,
    pub options_revision: u64,
}

impl StreamConfig {
    fn validate(&self) -> Result<(), CompositionError> {
        if [
            &self.capture_service,
            &self.encoder_service,
            &self.output_route,
        ]
        .into_iter()
        .any(|name| {
            name.is_empty()
                || name.len() > 128
                || name.chars().any(|character| character.is_control())
        }) {
            return Err(CompositionError::InvalidIdentity);
        }
        Ok(())
    }
}

/// One complete encoded access unit copied into extension-owned memory.
pub struct OwnedAccessUnit {
    pub bytes: Vec<u8>,
    pub sequence: u64,
    pub timestamp_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineFailure {
    Unavailable,
    CleanupUnconfirmed,
}

/// One pipeline owns its capture, encoder and release path. `stop` must be
/// idempotent: an uncertain result can be retried, and Drop is best effort.
pub trait StreamPipeline {
    fn produce(&mut self) -> Result<Option<OwnedAccessUnit>, PipelineFailure>;
    fn stop(&mut self) -> Result<(), PipelineFailure>;
}

pub struct RoutedAccessUnit<'a> {
    pub config: &'a StreamConfig,
    pub bytes: &'a [u8],
    pub sequence: u64,
    pub timestamp_us: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Admission {
    Accepted,
    Busy,
    /// The consumer may have received the bytes. Never replay this packet.
    Uncertain,
}

/// A caller-supplied host output. It may encode a new protocol, write a file,
/// or forward through a separately authorized transport. This trait does not
/// claim any unchanged Moonlight client can decode multiple routes.
pub trait PacketDemux {
    fn admit(&mut self, packet: RoutedAccessUnit<'_>) -> Admission;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionError {
    InvalidIdentity,
    DuplicateStream,
    StreamLimit,
    NotFound,
    CleanupUnconfirmed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamState {
    Running,
    Faulted,
    Closing,
}

#[derive(Default, Debug, Eq, PartialEq)]
pub struct TickReport {
    pub accepted: Vec<StreamId>,
    pub busy: Vec<StreamId>,
    pub failed: Vec<StreamId>,
}

struct Leg {
    config: StreamConfig,
    pipeline: Box<dyn StreamPipeline>,
    pending: Option<OwnedAccessUnit>,
    state: StreamState,
}

/// Single-threaded, bounded, fair-per-leg pump. No callback runs while a
/// borrowed frame or a cross-module lock is held by this utility. A busy
/// output retains at most one complete owned packet per leg and does not pull
/// another frame from that leg. Other legs continue independently.
#[derive(Default)]
pub struct StreamComposition {
    legs: BTreeMap<StreamId, Leg>,
}

impl StreamComposition {
    /// Validate identity and capacity before constructing an owning pipeline.
    /// Rejected admission never creates a native resource owner.
    pub fn insert_with(
        &mut self,
        config: StreamConfig,
        create: impl FnOnce() -> Box<dyn StreamPipeline>,
    ) -> Result<(), CompositionError> {
        config.validate()?;
        if self.legs.contains_key(&config.id) {
            return Err(CompositionError::DuplicateStream);
        }
        if self.legs.len() >= MAX_COMPOSED_STREAMS {
            return Err(CompositionError::StreamLimit);
        }
        self.legs.insert(
            config.id,
            Leg {
                config,
                pipeline: create(),
                pending: None,
                state: StreamState::Running,
            },
        );
        Ok(())
    }

    pub fn config(&self, id: StreamId) -> Option<&StreamConfig> {
        self.legs.get(&id).map(|leg| &leg.config)
    }

    pub fn state(&self, id: StreamId) -> Option<StreamState> {
        self.legs.get(&id).map(|leg| leg.state)
    }

    pub fn tick(&mut self, output: &mut dyn PacketDemux) -> TickReport {
        let mut report = TickReport::default();
        for (&id, leg) in &mut self.legs {
            if leg.state != StreamState::Running {
                continue;
            }
            if leg.pending.is_none() {
                match leg.pipeline.produce() {
                    Ok(Some(packet))
                        if !packet.bytes.is_empty()
                            && packet.bytes.len() <= MAX_COMPOSED_PACKET_BYTES =>
                    {
                        leg.pending = Some(packet);
                    }
                    Ok(None) => continue,
                    _ => {
                        leg.state = StreamState::Faulted;
                        report.failed.push(id);
                        continue;
                    }
                }
            }
            let packet = leg.pending.as_ref().expect("pending packet");
            match output.admit(RoutedAccessUnit {
                config: &leg.config,
                bytes: &packet.bytes,
                sequence: packet.sequence,
                timestamp_us: packet.timestamp_us,
            }) {
                Admission::Accepted => {
                    leg.pending = None;
                    report.accepted.push(id);
                }
                Admission::Busy => report.busy.push(id),
                Admission::Uncertain => {
                    // The output may have admitted it. Never replay under a
                    // fresh operation/sequence; fail only this route closed.
                    leg.pending = None;
                    leg.state = StreamState::Faulted;
                    report.failed.push(id);
                }
            }
        }
        report
    }

    /// Confirm one leg's cleanup before retiring its identity. A failed close
    /// retains the leg and forbids additional produce calls; callers must retry
    /// or use the provider's exact broker-owned recovery contract before unload.
    pub fn stop(&mut self, id: StreamId) -> Result<(), CompositionError> {
        let leg = self.legs.get_mut(&id).ok_or(CompositionError::NotFound)?;
        leg.state = StreamState::Closing;
        leg.pending = None;
        leg.pipeline
            .stop()
            .map_err(|_| CompositionError::CleanupUnconfirmed)?;
        self.legs.remove(&id);
        Ok(())
    }

    /// Attempt every close; failed legs remain owned here for an exact retry.
    /// A caller must not unload native provider code while the returned list
    /// is nonempty.
    pub fn try_shutdown(&mut self) -> Vec<StreamId> {
        let ids: Vec<_> = self.legs.keys().copied().collect();
        ids.into_iter()
            .filter(|id| self.stop(*id).is_err())
            .collect()
    }

    /// Transfer any unconfirmed owners to a longer-lived recovery coordinator
    /// without invoking Drop's best-effort close. The receiver is responsible
    /// for exact broker reconciliation and retaining provider code as needed.
    pub fn into_owned(mut self) -> Vec<OwnedPipeline> {
        std::mem::take(&mut self.legs)
            .into_values()
            .map(|leg| OwnedPipeline {
                config: leg.config,
                state: leg.state,
                pipeline: leg.pipeline,
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.legs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.legs.is_empty()
    }
}

pub struct OwnedPipeline {
    pub config: StreamConfig,
    pub state: StreamState,
    pub pipeline: Box<dyn StreamPipeline>,
}

impl Drop for StreamComposition {
    fn drop(&mut self) {
        // This is only best effort, never a shutdown barrier. Callers that
        // cannot confirm stop must use into_owned and retain native providers.
        for leg in self.legs.values_mut() {
            let _ = leg.pipeline.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct Source {
        route: u8,
        next: u64,
        attempts: Arc<Mutex<Vec<(u8, &'static str)>>>,
        close_fail_once: bool,
    }

    impl StreamPipeline for Source {
        fn produce(&mut self) -> Result<Option<OwnedAccessUnit>, PipelineFailure> {
            self.attempts.lock().unwrap().push((self.route, "produce"));
            self.next += 1;
            Ok(Some(OwnedAccessUnit {
                bytes: vec![self.route, self.next as u8],
                sequence: self.next,
                timestamp_us: self.next * 1_000,
            }))
        }

        fn stop(&mut self) -> Result<(), PipelineFailure> {
            self.attempts.lock().unwrap().push((self.route, "stop"));
            if self.close_fail_once {
                self.close_fail_once = false;
                Err(PipelineFailure::CleanupUnconfirmed)
            } else {
                Ok(())
            }
        }
    }

    #[derive(Default)]
    struct Sink {
        packets: Vec<(u32, String, Vec<u8>, u64)>,
        busy_once: Option<StreamId>,
        uncertain: Option<StreamId>,
    }

    impl PacketDemux for Sink {
        fn admit(&mut self, packet: RoutedAccessUnit<'_>) -> Admission {
            if self.busy_once == Some(packet.config.id) {
                self.busy_once = None;
                return Admission::Busy;
            }
            if self.uncertain == Some(packet.config.id) {
                return Admission::Uncertain;
            }
            self.packets.push((
                packet.config.id.get(),
                packet.config.output_route.clone(),
                packet.bytes.to_vec(),
                packet.sequence,
            ));
            Admission::Accepted
        }
    }

    fn config(id: u32, output: &str, revision: u64) -> StreamConfig {
        StreamConfig {
            id: StreamId::new(id).unwrap(),
            capture_service: format!("example.capture-{id}/1"),
            encoder_service: format!("example.encode-{id}/1"),
            output_route: output.into(),
            options_revision: revision,
        }
    }

    fn source(route: u8, trace: &Arc<Mutex<Vec<(u8, &'static str)>>>) -> Box<dyn StreamPipeline> {
        Box::new(Source {
            route,
            next: 0,
            attempts: trace.clone(),
            close_fail_once: false,
        })
    }

    #[test]
    fn two_owned_streams_demux_without_cross_talk_or_backpressure_coupling() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut composition = StreamComposition::default();
        let first = StreamId::new(1).unwrap();
        let second = StreamId::new(2).unwrap();
        composition
            .insert_with(config(1, "output-a", 7), || source(1, &trace))
            .unwrap();
        composition
            .insert_with(config(2, "output-b", 19), || source(2, &trace))
            .unwrap();
        let mut sink = Sink {
            busy_once: Some(first),
            ..Sink::default()
        };
        assert_eq!(
            composition.tick(&mut sink),
            TickReport {
                accepted: vec![second],
                busy: vec![first],
                failed: vec![],
            }
        );
        assert_eq!(sink.packets, [(2, "output-b".into(), vec![2, 1], 1)]);
        assert_eq!(composition.config(first).unwrap().options_revision, 7);
        assert_eq!(composition.config(second).unwrap().options_revision, 19);
        assert_eq!(composition.tick(&mut sink).accepted, vec![first, second]);
        assert_eq!(sink.packets[1], (1, "output-a".into(), vec![1, 1], 1));
        assert_eq!(sink.packets[2], (2, "output-b".into(), vec![2, 2], 2));
        assert_eq!(
            trace
                .lock()
                .unwrap()
                .iter()
                .filter(|event| **event == (1, "produce"))
                .count(),
            1
        );
        composition.stop(first).unwrap();
        assert_eq!(composition.len(), 1);
        assert_eq!(composition.tick(&mut sink).accepted, vec![second]);
        assert_eq!(sink.packets.last().unwrap().2, vec![2, 3]);
    }

    #[test]
    fn uncertain_output_fails_only_its_route_and_close_retries_exact_leg() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut composition = StreamComposition::default();
        let first = StreamId::new(1).unwrap();
        let second = StreamId::new(2).unwrap();
        composition
            .insert_with(config(1, "a", 1), || {
                Box::new(Source {
                    route: 1,
                    next: 0,
                    attempts: trace.clone(),
                    close_fail_once: true,
                })
            })
            .unwrap();
        composition
            .insert_with(config(2, "b", 2), || source(2, &trace))
            .unwrap();
        let mut sink = Sink {
            uncertain: Some(first),
            ..Sink::default()
        };
        let report = composition.tick(&mut sink);
        assert_eq!(report.failed, vec![first]);
        assert_eq!(report.accepted, vec![second]);
        assert_eq!(composition.tick(&mut sink).accepted, vec![second]);
        assert_eq!(composition.state(first), Some(StreamState::Faulted));
        assert_eq!(
            composition.stop(first),
            Err(CompositionError::CleanupUnconfirmed)
        );
        assert_eq!(composition.state(first), Some(StreamState::Closing));
        assert_eq!(composition.tick(&mut sink).accepted, vec![second]);
        composition.stop(first).unwrap();
        assert_eq!(composition.state(first), None);
        assert_eq!(composition.len(), 1);
        drop(composition);
        assert!(trace.lock().unwrap().contains(&(2, "stop")));
    }

    #[test]
    fn bounds_and_configuration_identity_fail_closed() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut composition = StreamComposition::default();
        assert_eq!(StreamId::new(0), Err(CompositionError::InvalidIdentity));
        for id in 1..=4 {
            composition
                .insert_with(config(id, "output", id as u64), || source(id as u8, &trace))
                .unwrap();
        }
        let mut rejected_created = false;
        assert_eq!(
            composition.insert_with(config(1, "output", 99), || {
                rejected_created = true;
                source(1, &trace)
            }),
            Err(CompositionError::DuplicateStream)
        );
        assert!(!rejected_created);
        assert_eq!(
            composition.insert_with(config(5, "output", 5), || source(5, &trace)),
            Err(CompositionError::StreamLimit)
        );
        let mut config = config(6, "bad\nroute", 0);
        composition.stop(StreamId::new(4).unwrap()).unwrap();
        assert_eq!(
            composition.insert_with(config.clone(), || source(6, &trace)),
            Err(CompositionError::InvalidIdentity)
        );
        config.output_route = "safe".into();
        composition
            .insert_with(config, || source(6, &trace))
            .unwrap();
    }

    #[test]
    fn failed_shutdown_transfers_unconfirmed_owner_for_later_recovery() {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut composition = StreamComposition::default();
        let id = StreamId::new(1).unwrap();
        composition
            .insert_with(config(1, "a", 3), || {
                Box::new(Source {
                    route: 1,
                    next: 0,
                    attempts: trace.clone(),
                    close_fail_once: true,
                })
            })
            .unwrap();
        assert_eq!(composition.try_shutdown(), vec![id]);
        assert_eq!(composition.state(id), Some(StreamState::Closing));
        let mut owners = composition.into_owned();
        assert_eq!(owners.len(), 1);
        assert_eq!(owners[0].config.options_revision, 3);
        assert_eq!(owners[0].state, StreamState::Closing);
        owners[0].pipeline.stop().unwrap();
        assert_eq!(
            trace
                .lock()
                .unwrap()
                .iter()
                .filter(|event| **event == (1, "stop"))
                .count(),
            2
        );
    }
}
