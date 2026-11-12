# Building the SDK

Rust source, crate manifests, C headers/tests, examples and guides retain their
committed core bytes. The root workspace, lockfile and export documents are
generated. The full local dependency closure includes optional and platform
dependencies and the runtime/testkit conformance libraries. Product implementations
are excluded. Four module API metadata files are included as runtime test fixtures;
their repository revisions and hashes are recorded in SDK_EXPORT.json. Guides
can reference core integration files outside this snapshot.

Use Rust 1.89 or newer and the platform prerequisites for native tests.

```sh
cargo fmt --all -- --check
cargo test --workspace --locked --offline
cargo clippy --workspace --all-targets --locked --offline -- -D warnings
```

The `network-test-support`, `network-http` and `process` SDK features are opt-in.
Default socket tests bind loopback. Ignored artifact, remote-binding, hardware
and platform tests require their own explicit qualification. Compiling on Linux
does not qualify Windows or macOS execution.

Compile C layout tests under `sdk/tests` with `-std=c11 -Wall -Wextra -Werror
-Isdk/include`. The logging test can also run as an executable. Build output
belongs under the selected target directory. The service-guard example uses
the current testkit; independent processor examples live under `sdk/examples`.

A consumer can use the canonical repository with an exact reviewed Git pin:

```toml
[dependencies]
amaterasu-module-sdk = { git = "https://github.com/nonary-streams/amaterasu-sdk", rev = "<reviewed-sdk-commit>" }
```

Check interface identity, table size and versions; preserve cancellation and
actual resource-retirement barriers. A provider claim or local queue admission
is not evidence of remote delivery, hardware support or installed behavior.

## Refreshing this snapshot

The crates, headers, examples and guides here are exported from
[Amaterasu](https://github.com/nonary-streams/amaterasu); edit them there. From
a core checkout, `python3 scripts/export_sdk.py PATH_TO_THIS_CHECKOUT` copies
the selection recorded in `SDK_EXPORT.json` from the core commit it is given,
and rewrites the root manifest and the record. `--seed CRATE` adds a crate and
everything it depends on; `--check` reports differences without writing.
Update `Cargo.lock` with `cargo metadata` afterwards and run the checks above.

Continuous integration runs those checks and the C header tests on Linux,
Windows and macOS. Pushing a tag `vX.Y.Z` that matches the workspace version
publishes the tagged source as a release once they pass. Module repositories
name the commit they build against in their `sdk-revision` file.
