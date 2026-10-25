# Amaterasu module SDK: lifecycle hooks

Amaterasu loads signed native modules through the versioned C bootstrap ABI in
`amaterasu-abi`. Native modules are trusted code; grants constrain runtime
messages and do **not** sandbox the module. The SDK exposes two different,
explicitly bounded ways to observe lifecycle facts:

| Path | Existing runtime service | Intended use | Can modify a frame? |
| --- | --- | --- | --- |
| Deferred observation | EventBus `Publish`/`Subscribe` | Status, metrics, diagnostics | No |
| Ordered control hook | `RegisterHook`/`RunHook`/`HookResult` | Non-realtime policy proposals | No |

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
must opt in and publish a documented observation at a safe point. Zero-copy
frame access, GPU surfaces, mutable packet bytes, input rewrites and synchronous
before-encode transformations require separate, versioned native interfaces
with explicit borrowed-buffer lifetime, threading, cancellation and failure
isolation. Do not advertise those capabilities based on this control-plane API.
