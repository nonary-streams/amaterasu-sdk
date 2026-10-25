# Amaterasu module SDK: lifecycle hooks

Amaterasu loads signed native modules through the versioned C bootstrap ABI in
`amaterasu-abi`. Native modules are trusted code; grants constrain runtime
messages and do **not** sandbox the module. The SDK exposes two different,
explicitly bounded ways to observe lifecycle facts:

| Path | Existing runtime service | Intended use | Can modify a frame? |
| --- | --- | --- | --- |
| Deferred observation | EventBus `Publish`/`Subscribe` | Status, metrics, diagnostics | No |
| Ordered control hook | `RegisterHook`/`RunHook`/`HookResult` | Non-realtime policy proposals | No |
| Native CPU processor v1 | Exact signed `NativeRegister`/`NativeBind` service | Trusted, synchronous CPU frame processing and encoded-packet observation | CPU pixels only, with explicit owned replacement |

## Trusted native CPU processing

`amaterasu_media::processing::MediaProcessorApi` is a separate versioned C
table, not an EventBus message or a control hook. A signed package must grant
the provider `native_provide` and the consumer `native_use` for the same exact
namespaced service and load the provider before its consumer. The provider
registers one factory with `NativeRegister`; the consumer requests the v1.0
table with `NativeBind`, which checks the live interface header and size.
Duplicate registrations are rejected. The API's existence does not grant a
module permission or enable a processor in a default product profile.

The table defines `PROCESS_BEFORE_FRAME` and `PROCESS_BEFORE_ENCODE` for
borrowed CPU BGRA8 or NV12 `CpuFrame` inputs. `PROCESS_PASS` preserves the
input, `PROCESS_DROP` discards that CPU frame, and `PROCESS_REPLACE` writes
exactly one borrowed output through `FrameSink`. The SDK copies that output
into `OwnedCpuFrame` before the callback returns. Geometry, format, stride,
timestamp and sequence must remain unchanged; output size is checked with
the existing `frame_size` bound of 128 MiB. A malformed decision, extra sink
call, shape change, failed callback or over-budget call fails closed. A
consumer may use `OwnedCpuFrame::borrowed()` only while keeping its owned
bytes alive through the downstream synchronous submit.

`PROCESS_AFTER_ENCODE` is observation-only in v1. The processor receives a
borrowed, validated `EncodedPacket` and must choose PASS without a packet sink
call. REPLACE and DROP are rejected: rewriting encoded bytes safely requires
codec-specific parameter-set and access-unit validation. DMA-BUF/GPU frames
have no entry point here. A consumer must pass them unchanged or reject an
explicit policy that requires a CPU transform; this ABI does not imply GPU
zero-copy support.

SDK calls are serialized on the consumer's module thread, must not reenter the
same binding, and must finish before releasing the binding or unloading its
provider. Native callbacks must not retain borrowed input/sink pointers or
unwind. `budget_us` is limited to 1–5,000 microseconds and measured only
after the call returns. It detects an overrun but cannot interrupt blocked
native code, so a caller must not promise a hard realtime deadline. An
overrun or malformed result should disable the processor or fail the stream
according to the signed product policy; it must not silently substitute an
unapproved transform. Existing media behavior is unchanged until a consumer
explicitly binds and calls this table.

The installed, verified runtime policy specifies the hook owner, ordered
participants, timeout and per-principal hook grant. A publisher's manifest or
extension cannot self-grant access. `QueryHook` discovers one authorized hook;
it does not enumerate the system. `UnregisterHook` must succeed before a
participant unloads. It returns `busy` while an invocation still references
that participant, including one queued behind an earlier participant.

## Typed messages

`amaterasu_contracts::hooks::HookEvent` is schema `1.0` with a lifecycle
`HookPoint`, a matching `HookSubject`, and `HookMode`. The SDK rejects malformed
versions, stage/subject mismatches and events larger than 8 KiB. The subject
contains bounded metadata: dimensions, pixel format, timestamp, packet length,
network length, event count or audio shape. `FrameMemory::GpuOpaque` is a label,
not a GPU handle. No bytes, raw pointers, allocations or borrowed lifetimes are
serializable through this contract.
On the EventBus path, `Before*` identifies when a fact was recorded; delivery
occurs later, after the producer returns to its bounded drain/tick. It is not
a synchronous interception point or veto. The default signed profile grants
no lifecycle observation topic to extensions.
`event_id` correlates a request and its reply within one producer epoch; IDs
may have gaps. EventBus publisher sequence and resync, not `event_id`, indicate
delivery loss.

For example, a video module can publish a deferred observation after its
existing frame operation has completed:

```rust,ignore
use amaterasu_module_sdk::{
    FrameMemory, HookEvent, HookMode, HookPoint, HookSubject, HookVersion,
    PixelFormat, RuntimeClient,
};

fn report_frame(runtime: &RuntimeClient, id: u64, width: u32, height: u32) {
    let event = HookEvent {
        version: HookVersion::CURRENT,
        event_id: id,
        point: HookPoint::AfterFrame,
        mode: HookMode::Observe,
        subject: HookSubject::VideoFrame {
            width, height, format: PixelFormat::Nv12,
            memory: FrameMemory::CpuOwned, timestamp_100ns: 0,
        },
    };
    // Best effort; the signed publish grant and bounded subscription queue
    // decide whether any extension receives it.
    let _ = runtime.publish_lifecycle_event("video.lifecycle/1", &event);
}
```

The owner may instead call `begin_lifecycle_hook` from a bounded module tick,
then poll `poll_lifecycle_hook` over subsequent ticks. Participants use the
ordinary broker `Poll`/`Complete` path and
`decode_lifecycle_event`/`encode_lifecycle_reply`. Installed participant order
is preserved. An observation can only receive `Continue`; `ProposeCancel` is
possible only when the owner explicitly marks the event `Propose`. The broker
returns proposals; it never cancels capture, drops a frame, rewrites an input
event or applies media transformations automatically. The owner must validate
the proposed action against its own signed policy and lifecycle state.
If the owner abandons an invocation, it must call `cancel_lifecycle_hook`
explicitly. Dropping the local Rust token has no broker side effect. A
successful owner cancellation releases the active request and queued
participant references so their modules can unregister. Cancellation of an
already completed or unknown invocation is reported as an error.

## Threading, time and unload

- Do not invoke the control hook broker synchronously from a capture, encode,
  transport, audio or input callback. A participant can take the installed
  deadline, up to 30 seconds. Publish only from a non-realtime drain/tick when
  the owner has opted in.
- The event broker has bounded subscriber queues; a full queue reports failure
  or its configured coalescing behavior. The SDK does not spin, block waiting
  for a subscriber, or retry publication.
- A control hook invocation is bounded by the installed timeout and participant
  cap. Its owner polls or explicitly cancels. At the deadline the broker
  retires the invocation, cancels/releases the active participant request,
  and retains a bounded owner-scoped timeout result for late polling. A
  participant must complete only the request it received from the broker.
- Unregistration is an explicit unload barrier. Keep callback code and context
  alive on `busy`; it succeeds once the invocation has completed, been
  cancelled, or reached its deadline. The module ABI's `destroy` must still
  join module-owned threads and release callbacks before library unload. The
  broker cannot interrupt a native callback or safely force-unload its code.

## Boundaries still to implement

The typed contract is not, by itself, before/after producer wiring. Each module
must opt in and publish a documented observation at a safe point. The native
CPU processor above is a separate contract for synchronous pixel processing;
it does not follow from these control-plane messages. GPU surfaces, mutable
encoded packet bytes and input rewrites still need their own contracts.

## Replacement providers and extension options

Observation hooks and replacement providers are different contracts. A hook
receives copied metadata and never owns capture. A replacement capture provider
implements the existing `NativeServiceFactory` and versioned
`amaterasu_media::CaptureApi` table under its own namespaced service. The
broker already permits one factory per exact service and checks
`native_provide`/`native_use` grants; registration cannot replace a live
factory. `NativeBinding` keeps a provider table/context alive until release.

`amaterasu_contracts::extensions::capture` adds an additive **planning**
contract. A provider descriptor names an existing video slot (for example
`pipewire`), component principal, exact namespaced service, interface version,
and declared capabilities. A separate `CaptureProviderApproval` must be
derived from verified host/package policy; it supplies the signed priority,
service, allowed capabilities and optional configuration identity. The helper
`plan_capture_provider` checks exact identity/service/component pairing,
provider and consumer native grants, ABI floor, capabilities and selection.
Its default `Builtin` choice returns no extension plan. `Exact` never silently
falls back; `Automatic` orders compatible candidates by signed priority then
identity. The result is an exact `NativeBind` request plan, **not** a registry,
new grant, loaded module or successful capture session.

The real bind must still check the returned table header and size before any
callback. `BorrowedDmaBuf` requires the capture v1.4 ABI and means descriptors
are borrowed only through the synchronous callback. A consumer retaining them
must duplicate FDs and respect acquire fences and compositor reuse; the
descriptor does not prove an end-to-end GPU zero-copy encode path. A provider
must not call back after its read returns, reenter a stream concurrently, or
unload while a binding/callback remains. Release consumers and their buffers,
then provider bindings, then module-owned threads and factory code. A failed
callback or partial bind fails the selected session closed; the planner does
not retry a different provider behind an explicit choice.

Extension settings use the existing caller-scoped
`amaterasu.configuration/1` service. `ExtensionConfigurationRef` binds the
approved option identity to that same component principal and formats its
existing `register`, `get` and revisioned `update` operations. Configuration
remains authoritative for schema types, bounds, defaults, migration, CAS and
persistence. No parallel settings store, validation language or browser route
is introduced. An uncertain write requires fresh readback; do not replay it.

Video has an exact signed-settings override for the X11 capture slot; the
general provider planning contract does not yet wire every slot or expose
extension options in Management. It does not enable
input, audio, network or UI replacement. A replacement UI needs the separately
verified signed web-bundle path and its own trust/product decision. No default
profile grants or runtime registrations change by adding these types.
