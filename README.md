# Amaterasu public SDK

This standalone source snapshot exports the public SDK from Amaterasu core
commit `66b67d053ccb27ac127d779a76a530e88d71ee8c`, based on public SDK commit
`b6197e6145fb1f1adb67566e512a1210544b213b`. Extension authors can build against
these libraries without the core source repository.

The workspace has eleven crates containing the bootstrap ABI, versioned control and
extension contracts, native media interfaces, module adapters, bounded data
plane, configuration schema, storage helpers, network interface and optional
network test implementation. `sdk/include` contains the public C headers;
`sdk/tests` contains their layout and logging checks. SDK guides are in `docs`.
`SDK_EXPORT.json` records the complete dependency closure, source file hashes,
source revision and exclusions.

`windows-capture-geometry` is an explicit native provider helper for Windows
output verification. Its platform details stay at the native boundary; generic
consumer contracts continue to use opaque targets. Scoped display status now
requires explicit reference ownership before a consumer can prove absence.

This revision intentionally follows the current core API rather than combining
older SDK snapshots. Historical `control`, `composition` and hook APIs absent
from that source are not restored. The old control-client QA fixture and an
example whose tests require core runtime/launch code are excluded. The standalone
application-library v2 and replacement examples remain available.

Pin dependencies to the exact reviewed SDK Git commit and commit the consumer's
lockfile. Package versions remain `0.1.0`; callers must check interface identity,
major/minor versions, table size, capabilities and ownership before invoking a
native table. Native modules are trusted code. API grants do not sandbox them.

See [DEVELOPING.md](DEVELOPING.md) for validation and consumption. The retained
[LICENSE](LICENSE) is MIT. This local source snapshot does not publish a release,
install a package or qualify native devices and playback.
