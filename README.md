# Amaterasu public SDK

This source snapshot exports committed Amaterasu core `72eaf63f0a381c1118656d615ae6b4badd50b86f`.
`SDK_EXPORT.json` records the exact source revision, dependency closure, exclusions
and file hashes. No uncommitted host or provider edits are included.

The workspace contains eighteen library crates and the service-guard example.
It includes current processing composition, provider adapters, tracing, bounded
requests, client entitlements, transfer channels, frame metadata, profile and
framing contracts. Runtime and module-testkit are reusable conformance libraries;
no product host, installed provider, private state or package is included.

Native plugins are trusted code. Select providers through approved composition,
grants, dependencies, exact interface identity and supported versions.
Optional HTTP upgrades, custom codec negotiation and dynamic HDR client playback
remain unqualified; their SDK helpers do not enable those production routes.

See [DEVELOPING.md](DEVELOPING.md) for checks and consumption. Pin dependencies to
an exact reviewed SDK commit; package versions remain `0.1.0`. Source publication is separate from signing,
installation, hardware qualification and client playback.
