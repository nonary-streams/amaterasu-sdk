# Building and consuming the SDK

This snapshot comes from core commit
`66b67d053ccb27ac127d779a76a530e88d71ee8c` and SDK base
`b6197e6145fb1f1adb67566e512a1210544b213b`. Exported Rust source, standalone
examples, C headers, C tests and guides retain their source bytes. Only the root
workspace wrapper, standalone lockfile and export documentation are generated.
Guides retain references to core integration modules and fixtures that are not
part of this standalone export.
`SDK_EXPORT.json` lists every exported source file and its SHA-256 hash.

The six explicit seed crates are `abi`, `contracts`, `module-sdk`, `media`,
`storage` and `windows-capture-geometry`. Their full dependency closure, including
optional and platform-specific paths, adds `configuration-schema`, `data-plane`,
`extension-api`, `network-api` and `network-sockets`. All eleven workspace crates
use relative paths within this repository. No core host, runtime or product
provider source is needed.

The Windows geometry crate is a reusable helper for native capture providers.
Use its OS details only at that platform boundary; generic consumer contracts
continue to exchange opaque capture targets. Its Windows dependency features
are declared locally, including DXGI Common, so checking it alone does not rely
on feature unification with a host capture provider.

Rust 1.89 or newer, a supported platform toolchain and cached registry
dependencies are required for offline builds. From this repository run:

```sh
cargo fmt --all -- --check
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
cargo test --workspace --all-features --locked --offline
cargo clippy --workspace --all-targets --all-features --locked --offline -- -D warnings
```

`network-test-support` enables the socket-backed test implementation;
`network-http` enables HTTP over the selected network interface. Default socket
tests bind loopback. Ignored remote/wildcard binding tests remain opt-in.

With the Windows target installed, validate the native helper independently:

```sh
cargo check --target x86_64-pc-windows-gnu -p amaterasu-windows-capture-geometry --lib --locked --offline
```

This cross-check verifies compilation; it does not exercise a Windows desktop.

Compile each `sdk/tests/*.c` with a C11 compiler, `-Wall -Wextra -Werror`, and
`-Isdk/include`. Layout tests compile as objects; `sdk/tests/logging.c` also
builds and runs as an executable. Put all build output under `target`.

An independent extension should declare the canonical SDK repository and pin an
exact reviewed SDK commit, then commit its own lockfile:

```toml
[dependencies]
amaterasu-module-sdk = { git = "https://github.com/nonary-streams/amaterasu-sdk", rev = "<reviewed-sdk-commit>" }
```

A local Git bundle or command-scoped Git source override can supply an unpublished
candidate while preserving the canonical dependency URL. The candidate must be
available locally at the pinned revision. Do not edit Cargo's cached source or
add machine-specific absolute paths to checked-in manifests.

Use `ManagedModule` and the SDK export macro for the bootstrap entry point.
Select providers through verified startup composition and exact grants and
dependencies. Check interface family, versions, table size and capabilities;
preserve callback lifetimes, cancellation and resource-retirement barriers.
The SDK's capture, display-target, session-presentation, input and virtual-display
adapters do not confer permission to create native resources or bypass the host.

This export intentionally drops historical APIs absent from the current core
tree. It does not assert compatibility with earlier SDK snapshots. The removed
control-client QA fixture depended on such an API. The omitted
`application-library-provider` example has core runtime and launch module test
dependencies; its standalone v2 and replacement peers remain byte-exact.

The host remains responsible for verified signatures, publisher approval,
component dependencies and policy. Native modules remain trusted code. Passing
these source and ABI layout checks does not establish package activation,
hardware capture, client playback or publication.
