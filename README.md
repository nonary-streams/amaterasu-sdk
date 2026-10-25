# Amaterasu SDK foundation

This source repository contains the version 1 C ABI, broker control contracts,
native module SDK, storage helpers and media interface tables. It is prepared
for independent source builds; it is not an installed host release. Amaterasu
core and SDK are maintained by the project owner. Extension authors use this
SDK from their own repositories and do not need access to Amaterasu's source
repositories or to republish the host.

The package versions are `0.1.0`; ABI compatibility must still be checked by
`AbiHeader` major/minor/size and each native interface version. See
`DEVELOPING.md` for source builds and supported extension boundaries. Native
modules remain trusted code; grants are not a sandbox. Project-authored source is MIT licensed. Release tags and installed-host
qualification remain separate.
