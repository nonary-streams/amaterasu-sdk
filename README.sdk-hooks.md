# Developer hook SDK source snapshot

This source snapshot copies tracked `crates/contracts/**` and
`crates/module-sdk/**` from the independently reviewed Amaterasu integration
commit `4d2b6a40674dac0c46955ffc46f7a162ba23c48e`. It retains ABI, media
and shared storage from foundation v0.2. The v0.1 and v0.2 source tags remain
unchanged. This repository revision is the first local extraction proof with
the typed/versioned developer hook contracts; earlier tags are pre-hook.

This source tag is not a signed package or deployed extension runtime. The
module and protocol source proofs need to pin this exact foundation revision
before claiming access to the new SDK. Their local `file://` URLs are temporary;
portable lockfiles require an approved stable Git or registry source location.
