# Private plugin control client

`amaterasu_module_sdk::control::ControlClient` is for a separately launched
control executable, using the private stdin/stdout handles supplied by its
host. It does not attach a native module to another principal's session or
discover a privileged broker. Keep diagnostic logs off the control stdout.
The host remains authoritative for approved publisher/component identity,
active composition, exact executable bytes, grants and request expiration.

The typed client supports permission request/status/invoke/cancel and plugin
executable elevation request/status/launch/cancel. There is no approve/deny API
or caller-supplied principal. Review and OS authorization are separate host
boundaries. An elevation approval refers to the plugin's exact reviewed
executable and direct arguments, with an empty environment and `/` working
directory on Linux. Windows accepts a narrow drive-absolute directory that
must exactly match the installed broker's protected working directory. This
lexical SDK check does not establish directory ownership; the broker must pin
and verify it. Neither platform permits the plugin to select an arbitrary
executable path.

Cancellation withdraws only the requesting child's pending or approved
unattempted request and reports `revoked`. Cancellation cannot undo an action.
Once an elevation launch is reserved, the request is spent, including while OS
consent is pending: plugin cancellation is rejected. Administrator revocation
uses a separate protected-broker coordination path. Cancellation audit failure
leaves the in-memory request revoked but returns an error; no successful
durable acknowledgement is fabricated.

Messages use bounded newline JSON. One mutable client makes at most one call
at a time. A write/read failure, malformed/oversized reply, or wrong request ID
retires the channel with `OutcomeUncertain`; subsequent calls return `Retired`
without emitting another request. Even a status read is not automatically
retried because a late reply could otherwise be consumed by a later call.
Local validation failure sends no bytes. A complete host rejection is returned
as `HostRejected` and does not desynchronize the channel. Applications must
never turn an uncertain launch outcome into an automatic retry.

The client performs synchronous reads; the host owns deadlines and teardown.
Do not call it on a host scheduler thread. Cancellation requires the matching
host protocol revision; an older host rejects the new operation with
`invalid_request`, with no fallback/replay. These interfaces do not install a
service, authorize an administrator, sandbox native code, or prove any OS
elevation adapter is installed.

`qa/control-client-consumer` is an independent Cargo workspace consuming only
this candidate SDK. Its default mode exercises typed state/encoding and lost
reply handling against typed synthetic responses. `--host-pipe` exercises the
actual SDK methods against a supplied host private pipe and cancels both kinds
of request without calling launch. Neither mode obtains approval authority or
performs privileged execution.
