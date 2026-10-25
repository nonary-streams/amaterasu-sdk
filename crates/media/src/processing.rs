//! Trusted, synchronous CPU media processing ABI. This is separate from the
//! deferred, metadata-only EventBus observation contract.
use crate::{CpuFrame, EncodedPacket, FrameSink, PacketSink};
use amaterasu_abi::{NativeInterface, Status};
use std::{ffi::c_void, mem::size_of};

pub const MEDIA_PROCESSOR_INTERFACE_MAJOR: u32 = 1;
pub const MEDIA_PROCESSOR_INTERFACE_MINOR: u32 = 0;

pub const PROCESS_BEFORE_FRAME: u32 = 1;
pub const PROCESS_BEFORE_ENCODE: u32 = 2;
pub const PROCESS_AFTER_ENCODE: u32 = 3;

pub const PROCESS_PASS: u32 = 0;
pub const PROCESS_REPLACE: u32 = 1;
pub const PROCESS_DROP: u32 = 2;

/// One bound, trusted processor. The consumer calls only on its serialized
/// module thread and never reenters the same processor. The provider must not
/// retain the input, sink, or output pointers after the call, invoke a sink
/// asynchronously, or unwind across this C boundary. Provider code and the
/// interface allocation remain live until all calls and bindings are released.
///
/// `process_frame` accepts BEFORE_FRAME or BEFORE_ENCODE on a validated CPU
/// frame. PASS and DROP call no sink. REPLACE calls `output.write` exactly once
/// with a fully initialized CPU frame whose geometry, format, stride, timestamp
/// and sequence equal the input's; the consumer copies bytes before returning.
/// The provider must set `decision` and return STATUS_OK. Any other status or
/// malformed result fails closed. DROP is not accepted after encoding.
///
/// `process_packet` accepts AFTER_ENCODE only, and is observation-only in v1:
/// it must return PASS without calling the packet sink. Changing encoded bytes
/// would need codec-aware parameter-set and access-unit validation. GPU frames
/// and DMA-BUF descriptors have no entry point in this table; consumers must
/// bypass them unchanged or reject a policy that requires a CPU transform.
///
/// A caller can measure a wall-clock budget after return and retire an overrun,
/// but cannot preempt native code stuck inside either callback. Unload must
/// wait for that call to finish; implementations may not claim hard deadlines.
#[repr(C)]
pub struct MediaProcessorApi {
    pub base: NativeInterface,
    pub process_frame:
        unsafe extern "C" fn(*mut c_void, u32, *const CpuFrame, FrameSink, *mut u32) -> Status,
    pub process_packet: unsafe extern "C" fn(
        *mut c_void,
        u32,
        *const EncodedPacket,
        PacketSink,
        *mut u32,
    ) -> Status,
}

pub const MEDIA_PROCESSOR_API_V1_SIZE: u32 = size_of::<MediaProcessorApi>() as u32;
