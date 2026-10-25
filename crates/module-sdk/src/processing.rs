//! Owned results for the trusted synchronous CPU processor ABI.
use crate::NativeBinding;
use amaterasu_abi::{STATUS_INVALID_ARGUMENT, STATUS_OK, Status};
use amaterasu_media::processing::{
    MEDIA_PROCESSOR_API_V1_SIZE, MEDIA_PROCESSOR_INTERFACE_MAJOR, MediaProcessorApi,
    PROCESS_AFTER_ENCODE, PROCESS_BEFORE_ENCODE, PROCESS_BEFORE_FRAME, PROCESS_DROP, PROCESS_PASS,
    PROCESS_REPLACE,
};
use amaterasu_media::{CpuFrame, EncodedPacket, FrameSink, PacketSink};
use std::{ffi::c_void, time::Instant};

/// A native callback cannot be preempted. The caller measures this budget only
/// after it returns and must stop using an over-budget processor for the stream.
pub const MAX_PROCESSOR_BUDGET_US: u32 = 5_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessorError {
    InvalidInput,
    InvalidInterface,
    InvalidResult,
    Backend(Status),
    OverBudget,
}

#[derive(Debug, Eq, PartialEq)]
pub struct OwnedCpuFrame {
    pub width: u32,
    pub height: u32,
    pub format: u32,
    pub stride: u32,
    pub timestamp_us: u64,
    pub sequence: u64,
    pub bytes: Vec<u8>,
}

impl OwnedCpuFrame {
    /// Borrow the owned pixels only while `self` stays immobile and alive.
    pub fn borrowed(&self) -> CpuFrame {
        CpuFrame {
            width: self.width,
            height: self.height,
            format: self.format,
            stride: self.stride,
            data: self.bytes.as_ptr(),
            len: self.bytes.len(),
            timestamp_us: self.timestamp_us,
            sequence: self.sequence,
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub enum FrameProcessOutcome {
    Pass,
    Drop,
    Replace(OwnedCpuFrame),
}

struct FrameCollector {
    input: CpuFrame,
    calls: u8,
    failed: bool,
    frame: Option<OwnedCpuFrame>,
}

unsafe extern "C" fn copy_frame(context: *mut c_void, output: *const CpuFrame) -> Status {
    if context.is_null() || output.is_null() {
        return STATUS_INVALID_ARGUMENT;
    }
    // SAFETY: the processor receives this stack collector and calls synchronously.
    let collector = unsafe { &mut *context.cast::<FrameCollector>() };
    collector.calls = collector.calls.saturating_add(1);
    if collector.calls != 1 {
        collector.failed = true;
        return STATUS_INVALID_ARGUMENT;
    }
    // SAFETY: the trusted provider guarantees a live initialized output table.
    let output = unsafe { &*output };
    if output.validate().is_err()
        || output.width != collector.input.width
        || output.height != collector.input.height
        || output.format != collector.input.format
        || output.stride != collector.input.stride
        || output.timestamp_us != collector.input.timestamp_us
        || output.sequence != collector.input.sequence
    {
        collector.failed = true;
        return STATUS_INVALID_ARGUMENT;
    }
    let mut bytes = Vec::new();
    if bytes.try_reserve_exact(output.len).is_err() {
        collector.failed = true;
        return STATUS_INVALID_ARGUMENT;
    }
    // SAFETY: the trusted provider keeps output bytes initialized and immutable
    // through this synchronous sink call; validate checked the bounded shape.
    let Ok(borrowed) = (unsafe { output.bytes() }) else {
        collector.failed = true;
        return STATUS_INVALID_ARGUMENT;
    };
    bytes.extend_from_slice(borrowed);
    collector.frame = Some(OwnedCpuFrame {
        width: output.width,
        height: output.height,
        format: output.format,
        stride: output.stride,
        timestamp_us: output.timestamp_us,
        sequence: output.sequence,
        bytes,
    });
    STATUS_OK
}

unsafe extern "C" fn reject_packet_output(context: *mut c_void, _: *const EncodedPacket) -> Status {
    if !context.is_null() {
        // SAFETY: caller supplies a live synchronous stack flag.
        unsafe { *context.cast::<bool>() = true };
    }
    STATUS_INVALID_ARGUMENT
}

fn valid_budget(budget_us: u32) -> bool {
    (1..=MAX_PROCESSOR_BUDGET_US).contains(&budget_us)
}

impl NativeBinding {
    /// Run an opt-in CPU stage and copy any replacement into caller-owned
    /// storage. `budget_us` is measured after the native call, not preemptive.
    ///
    /// # Safety
    /// This binding must be the exact MediaProcessorApi v1 service obtained via
    /// NativeBind; the input must point to initialized immutable bytes for this
    /// call. No callback may retain the input/sink, reenter this binding, or
    /// unwind. The caller serializes all calls and releases the binding only
    /// after they return. GPU/DMA-BUF input has no path through this function.
    pub unsafe fn process_cpu_frame(
        &self,
        stage: u32,
        input: &CpuFrame,
        budget_us: u32,
    ) -> Result<FrameProcessOutcome, ProcessorError> {
        // SAFETY: the caller guarantees the exact bound table and its lifetime.
        let api = unsafe { self.table::<MediaProcessorApi>() }
            .map_err(|_| ProcessorError::InvalidInterface)?;
        // SAFETY: the binding and input satisfy the same processor contract.
        unsafe { process_cpu_frame_api(api, stage, input, budget_us) }
    }

    /// Observe one already encoded packet. Version 1 permits PASS only.
    ///
    /// # Safety
    /// The binding must be an exact live MediaProcessorApi v1 service, and the
    /// packet bytes must remain initialized and immutable through this call.
    /// Calls are serialized and the processor must not retain pointers, reenter
    /// the binding, or unwind.
    pub unsafe fn observe_encoded_packet(
        &self,
        packet: &EncodedPacket,
        budget_us: u32,
    ) -> Result<(), ProcessorError> {
        // SAFETY: the caller guarantees the exact bound table and its lifetime.
        let api = unsafe { self.table::<MediaProcessorApi>() }
            .map_err(|_| ProcessorError::InvalidInterface)?;
        // SAFETY: the binding and input satisfy the same processor contract.
        unsafe { observe_encoded_packet_api(api, packet, budget_us) }
    }
}

/// Copy and validate one processor result. A provider may return borrowed
/// output only through the synchronous sink, never by transferring its allocator.
///
/// # Safety
/// `api` must be a live MediaProcessorApi v1 table, and `input` must reference
/// initialized immutable bytes through return. Calls are serialized; the
/// provider must not retain pointers, reenter or unwind.
pub unsafe fn process_cpu_frame_api(
    api: &MediaProcessorApi,
    stage: u32,
    input: &CpuFrame,
    budget_us: u32,
) -> Result<FrameProcessOutcome, ProcessorError> {
    if api.base.header.major != MEDIA_PROCESSOR_INTERFACE_MAJOR
        || api.base.header.size < MEDIA_PROCESSOR_API_V1_SIZE
    {
        return Err(ProcessorError::InvalidInterface);
    }
    if !matches!(stage, PROCESS_BEFORE_FRAME | PROCESS_BEFORE_ENCODE)
        || !valid_budget(budget_us)
        || input.validate().is_err()
    {
        return Err(ProcessorError::InvalidInput);
    }
    let mut collector = FrameCollector {
        input: *input,
        calls: 0,
        failed: false,
        frame: None,
    };
    let sink = FrameSink {
        context: (&mut collector as *mut FrameCollector).cast(),
        write: copy_frame,
    };
    let mut decision = u32::MAX;
    let started = Instant::now();
    // SAFETY: input and collector remain live; the provider obeys the ABI.
    let status =
        unsafe { (api.process_frame)(api.base.context, stage, input, sink, &mut decision) };
    if started.elapsed().as_micros() > u128::from(budget_us) {
        return Err(ProcessorError::OverBudget);
    }
    if status != STATUS_OK {
        return Err(ProcessorError::Backend(status));
    }
    if collector.failed {
        return Err(ProcessorError::InvalidResult);
    }
    match decision {
        PROCESS_PASS if collector.calls == 0 => Ok(FrameProcessOutcome::Pass),
        PROCESS_DROP if collector.calls == 0 => Ok(FrameProcessOutcome::Drop),
        PROCESS_REPLACE if collector.calls == 1 => collector
            .frame
            .map(FrameProcessOutcome::Replace)
            .ok_or(ProcessorError::InvalidResult),
        _ => Err(ProcessorError::InvalidResult),
    }
}

/// Observe one already encoded packet. Packet mutation is intentionally not
/// permitted until codec-aware validation is specified.
///
/// # Safety
/// `api` must be a live MediaProcessorApi v1 table and `packet` must reference
/// initialized immutable bytes through return. Calls are serialized; the
/// provider must not retain pointers, reenter or unwind.
pub unsafe fn observe_encoded_packet_api(
    api: &MediaProcessorApi,
    packet: &EncodedPacket,
    budget_us: u32,
) -> Result<(), ProcessorError> {
    if api.base.header.major != MEDIA_PROCESSOR_INTERFACE_MAJOR
        || api.base.header.size < MEDIA_PROCESSOR_API_V1_SIZE
    {
        return Err(ProcessorError::InvalidInterface);
    }
    if !valid_budget(budget_us) || packet.validate().is_err() {
        return Err(ProcessorError::InvalidInput);
    }
    let mut emitted = false;
    let sink = PacketSink {
        context: (&mut emitted as *mut bool).cast(),
        write: reject_packet_output,
    };
    let mut decision = u32::MAX;
    let started = Instant::now();
    // SAFETY: packet, sink and decision remain live until callback return.
    let status = unsafe {
        (api.process_packet)(
            api.base.context,
            PROCESS_AFTER_ENCODE,
            packet,
            sink,
            &mut decision,
        )
    };
    if started.elapsed().as_micros() > u128::from(budget_us) {
        return Err(ProcessorError::OverBudget);
    }
    if status != STATUS_OK {
        return Err(ProcessorError::Backend(status));
    }
    if emitted || decision != PROCESS_PASS {
        return Err(ProcessorError::InvalidResult);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use amaterasu_abi::{AbiHeader, NativeInterface};
    use amaterasu_media::{CODEC_H264, PIXEL_BGRA8};

    struct Fake {
        mode: u32,
        packet_calls: u32,
    }

    unsafe extern "C" fn frame(
        context: *mut c_void,
        _: u32,
        input: *const CpuFrame,
        sink: FrameSink,
        decision: *mut u32,
    ) -> Status {
        // SAFETY: each test call supplies live local context, input and decision.
        let (fake, input, decision) =
            unsafe { (&mut *context.cast::<Fake>(), &*input, &mut *decision) };
        *decision = match fake.mode {
            0 => PROCESS_PASS,
            1 => PROCESS_DROP,
            2..=4 => PROCESS_REPLACE,
            _ => u32::MAX,
        };
        if matches!(fake.mode, 2..=4) {
            // SAFETY: the test owns valid input bytes through this callback.
            let mut bytes = unsafe { input.bytes().unwrap() }.to_vec();
            bytes[0] = 91;
            let output = CpuFrame {
                width: if fake.mode == 4 {
                    input.width + 1
                } else {
                    input.width
                },
                data: bytes.as_ptr(),
                ..*input
            };
            // SAFETY: output and bytes remain live through each sink call.
            let _ = unsafe { sink.send(&output) };
            if fake.mode == 3 {
                let _ = unsafe { sink.send(&output) };
            }
        }
        STATUS_OK
    }

    unsafe extern "C" fn packet(
        context: *mut c_void,
        stage: u32,
        input: *const EncodedPacket,
        sink: PacketSink,
        decision: *mut u32,
    ) -> Status {
        // SAFETY: each test call supplies live local context, input and decision.
        let (fake, decision) = unsafe { (&mut *context.cast::<Fake>(), &mut *decision) };
        assert_eq!(stage, PROCESS_AFTER_ENCODE);
        fake.packet_calls += 1;
        *decision = if fake.mode == 5 {
            PROCESS_REPLACE
        } else {
            PROCESS_PASS
        };
        if fake.mode == 6 {
            // SAFETY: packet is live for this synchronous callback.
            let _ = unsafe { sink.send(&*input) };
        }
        STATUS_OK
    }

    fn api(fake: &mut Fake) -> MediaProcessorApi {
        MediaProcessorApi {
            base: NativeInterface {
                header: AbiHeader::of::<MediaProcessorApi>(),
                context: (fake as *mut Fake).cast(),
                release: None,
            },
            process_frame: frame,
            process_packet: packet,
        }
    }

    #[test]
    fn frame_processor_pass_drop_and_owned_replacement_are_distinct() {
        let bytes = [1u8; 16];
        let input = CpuFrame {
            width: 2,
            height: 2,
            format: PIXEL_BGRA8,
            stride: 8,
            data: bytes.as_ptr(),
            len: bytes.len(),
            timestamp_us: 17,
            sequence: 3,
        };
        let mut fake = Fake {
            mode: 0,
            packet_calls: 0,
        };
        let table = api(&mut fake);
        // SAFETY: all tables and borrowed bytes are local and live through calls.
        unsafe {
            assert_eq!(
                process_cpu_frame_api(&table, PROCESS_BEFORE_FRAME, &input, 5_000),
                Ok(FrameProcessOutcome::Pass)
            );
            fake.mode = 1;
            assert_eq!(
                process_cpu_frame_api(&table, PROCESS_BEFORE_ENCODE, &input, 5_000),
                Ok(FrameProcessOutcome::Drop)
            );
            fake.mode = 2;
            let FrameProcessOutcome::Replace(owned) =
                process_cpu_frame_api(&table, PROCESS_BEFORE_ENCODE, &input, 5_000).unwrap()
            else {
                panic!("expected owned replacement")
            };
            assert_eq!(owned.bytes[0], 91);
            assert_eq!(owned.borrowed().validate(), Ok(()));
            assert_eq!(owned.sequence, 3);
            fake.mode = 3;
            assert_eq!(
                process_cpu_frame_api(&table, PROCESS_BEFORE_FRAME, &input, 5_000),
                Err(ProcessorError::InvalidResult)
            );
            fake.mode = 4;
            assert_eq!(
                process_cpu_frame_api(&table, PROCESS_BEFORE_FRAME, &input, 5_000),
                Err(ProcessorError::InvalidResult)
            );
            fake.mode = 7;
            assert_eq!(
                process_cpu_frame_api(&table, PROCESS_BEFORE_FRAME, &input, 5_000),
                Err(ProcessorError::InvalidResult)
            );
            assert_eq!(
                process_cpu_frame_api(&table, PROCESS_AFTER_ENCODE, &input, 5_000),
                Err(ProcessorError::InvalidInput)
            );
            assert_eq!(
                process_cpu_frame_api(&table, PROCESS_BEFORE_FRAME, &input, 0),
                Err(ProcessorError::InvalidInput)
            );
        }
        let mut malformed = api(&mut fake);
        malformed.base.header.major += 1;
        // SAFETY: invalid header is rejected before the callback is reached.
        assert_eq!(
            unsafe { process_cpu_frame_api(&malformed, PROCESS_BEFORE_FRAME, &input, 5_000) },
            Err(ProcessorError::InvalidInterface)
        );
        assert_eq!(bytes[0], 1); // The caller's borrowed input was never mutated.
    }

    #[test]
    fn encoded_packet_stage_is_observation_only() {
        let bytes = [0, 0, 0, 1, 0x65];
        let input = EncodedPacket {
            codec: CODEC_H264,
            flags: 0,
            data: bytes.as_ptr(),
            len: bytes.len(),
            timestamp_us: 17,
            sequence: 3,
        };
        let mut fake = Fake {
            mode: 0,
            packet_calls: 0,
        };
        let table = api(&mut fake);
        // SAFETY: local table and packet bytes remain live for all calls.
        unsafe {
            assert_eq!(observe_encoded_packet_api(&table, &input, 5_000), Ok(()));
            fake.mode = 5;
            assert_eq!(
                observe_encoded_packet_api(&table, &input, 5_000),
                Err(ProcessorError::InvalidResult)
            );
            fake.mode = 6;
            assert_eq!(
                observe_encoded_packet_api(&table, &input, 5_000),
                Err(ProcessorError::InvalidResult)
            );
        }
        assert_eq!(fake.packet_calls, 3);
    }
}
