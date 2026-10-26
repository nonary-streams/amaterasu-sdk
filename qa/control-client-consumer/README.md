# Candidate SDK control consumer

This is a separate Cargo workspace, depending only on the candidate public SDK
and serde_json. Run `cargo run --manifest-path qa/control-client-consumer/Cargo.toml`.
It calls typed permission request/status/invoke/cancel and elevation
request/status/launch/cancel APIs, inspects the encoded typed requests, and
checks lost-reply retirement prevents a second launch write.

The replies are typed synthetic fixture responses, including administrator
approval states supplied by the test. This proves the external consumer and
SDK codec/client path, not installed host authentication, OS consent or elevated
execution. There is no SDK method to approve or assign another plugin identity.
