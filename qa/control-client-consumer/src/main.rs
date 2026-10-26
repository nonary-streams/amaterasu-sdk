//! Independent candidate-SDK consumer. Synthetic responses only; no host login
//! or approval authority is obtained and no process is elevated or launched.
use amaterasu_module_sdk::control::{
    Capability, ControlClient, ControlError, ControlReply, ControlRequest, ElevationOutcome,
    LaunchSpec, PermissionState,
};
use std::io::Cursor;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--host-pipe") {
        host_pipe();
        return;
    }
    let permission_id = "a".repeat(32);
    let elevation_id = "b".repeat(32);
    let messages = [
        reply(&permission_id, None, None, None),
        reply(&permission_id, Some(PermissionState::Pending), None, None),
        reply(&permission_id, Some(PermissionState::Approved), None, None),
        reply(&permission_id, None, None, None),
        reply(&permission_id, Some(PermissionState::Revoked), None, None),
        reply(
            &elevation_id,
            Some(PermissionState::Pending),
            Some(false),
            None,
        ),
        reply(
            &elevation_id,
            Some(PermissionState::Pending),
            Some(false),
            None,
        ),
        reply(
            &elevation_id,
            Some(PermissionState::Revoked),
            Some(false),
            None,
        ),
        reply(
            &elevation_id,
            Some(PermissionState::Pending),
            Some(false),
            None,
        ),
        reply(
            &elevation_id,
            Some(PermissionState::Approved),
            Some(false),
            None,
        ),
        reply(
            &elevation_id,
            Some(PermissionState::Approved),
            Some(true),
            Some(ElevationOutcome::AwaitingOsConsent),
        ),
        reply(
            &elevation_id,
            Some(PermissionState::Approved),
            Some(true),
            Some(ElevationOutcome::LaunchUnconfirmed),
        ),
    ];
    let mut responses = Vec::new();
    for message in messages {
        serde_json::to_writer(&mut responses, &message).unwrap();
        responses.push(b'\n');
    }
    let mut client = ControlClient::new(Cursor::new(responses), Vec::new());
    let capability = Capability {
        action: "demo.echo".into(),
        resource: "fixture".into(),
    };
    let requested = client
        .request_permission(capability.clone(), "Synthetic independent client".into())
        .unwrap();
    let id = requested.request_id.unwrap();
    assert_eq!(
        client.permission_status(id.clone()).unwrap().state,
        Some(PermissionState::Pending)
    );
    // This next fixture response represents an administrator decision elsewhere;
    // the requesting SDK client has no API that could make that decision.
    assert_eq!(
        client.permission_status(id.clone()).unwrap().state,
        Some(PermissionState::Approved)
    );
    client.invoke(id.clone(), capability).unwrap();
    assert_eq!(
        client.cancel_permission(id).unwrap().state,
        Some(PermissionState::Revoked)
    );
    let launch = LaunchSpec {
        arguments: vec!["--synthetic".into()],
        environment: Default::default(),
        cwd: "/".into(),
    };
    let requested = client
        .request_elevation(launch.clone(), "Synthetic code launch request".into())
        .unwrap();
    let id = requested.request_id.unwrap();
    assert_eq!(
        client.elevation_status(id.clone()).unwrap().state,
        Some(PermissionState::Pending)
    );
    assert_eq!(
        client.cancel_elevation(id).unwrap().state,
        Some(PermissionState::Revoked)
    );
    let requested = client
        .request_elevation(launch, "Second synthetic request".into())
        .unwrap();
    let id = requested.request_id.unwrap();
    assert_eq!(
        client.elevation_status(id.clone()).unwrap().state,
        Some(PermissionState::Approved)
    );
    assert_eq!(
        client
            .launch_elevated(id.clone())
            .unwrap()
            .elevation_outcome,
        Some(ElevationOutcome::AwaitingOsConsent)
    );
    assert_eq!(
        client
            .elevation_status(id.clone())
            .unwrap()
            .elevation_outcome,
        Some(ElevationOutcome::LaunchUnconfirmed)
    );
    // Lost reply: no retry is emitted and the channel stays retired.
    assert!(matches!(
        client.elevation_status(id.clone()),
        Err(ControlError::OutcomeUncertain(_))
    ));
    assert_eq!(client.launch_elevated(id), Err(ControlError::Retired));
    let (_, encoded) = client.into_parts();
    let operations: Vec<ControlRequest> = String::from_utf8(encoded)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(operations.len(), 13);
    assert!(matches!(operations[0], ControlRequest::Request { .. }));
    assert!(matches!(operations[4], ControlRequest::Cancel { .. }));
    assert!(matches!(
        operations[7],
        ControlRequest::CancelElevation { .. }
    ));
    assert_eq!(
        operations
            .iter()
            .filter(|request| matches!(request, ControlRequest::LaunchElevated { .. }))
            .count(),
        1
    );
    println!(
        "independent candidate SDK: permission/elevation states, cancellation and no-retry encoding passed"
    );
}

fn host_pipe() {
    let input = std::io::stdin();
    let output = std::io::stdout();
    let mut client = ControlClient::new(input.lock(), output.lock());
    let permission = client
        .request_permission(
            Capability {
                action: "demo.echo".into(),
                resource: "fixture".into(),
            },
            "Independent public SDK host-pipe test".into(),
        )
        .unwrap();
    let id = permission.request_id.unwrap();
    assert_eq!(
        client.permission_status(id.clone()).unwrap().state,
        Some(PermissionState::Pending)
    );
    assert_eq!(
        client.cancel_permission(id).unwrap().state,
        Some(PermissionState::Revoked)
    );
    let elevation = client
        .request_elevation(
            LaunchSpec {
                arguments: vec!["--synthetic".into()],
                environment: Default::default(),
                cwd: "/".into(),
            },
            "Only request and cancel; never launch".into(),
        )
        .unwrap();
    let id = elevation.request_id.unwrap();
    assert_eq!(
        client.elevation_status(id.clone()).unwrap().state,
        Some(PermissionState::Pending)
    );
    assert_eq!(
        client.cancel_elevation(id).unwrap().state,
        Some(PermissionState::Revoked)
    );
}

fn reply(
    id: &str,
    state: Option<PermissionState>,
    attempted: Option<bool>,
    elevation_outcome: Option<ElevationOutcome>,
) -> ControlReply {
    ControlReply {
        ok: true,
        code: "ok".into(),
        request_id: Some(id.into()),
        state,
        attempted,
        elevation_outcome,
    }
}
