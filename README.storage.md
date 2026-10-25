# Shared storage package in the foundation

`amaterasu-storage` is added from committed Amaterasu
`a6b0db9e36a1a59328ab4b4785b27b210f8182b7` without source changes.
Authentication, release, and the input companion all consume this general
purpose package; assigning it to the input repository would create conflicting
ownership. This v0.2 source snapshot preserves the v0.1 foundation commit and
tag for existing pinned audio module proofs.

The package version remains `0.1.0` to match the extracted source. The source
tag is an extraction milestone, not a released SDK or signed native artifact.
