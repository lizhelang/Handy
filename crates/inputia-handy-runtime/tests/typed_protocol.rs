use inputia_handy_runtime::voice_protocol::{
    HostTargetToken, TypedCaptureCommand, TypedCaptureRequest, VoicePeer, VoiceWireRequest,
};

fn peer() -> VoicePeer<'static> {
    VoicePeer {
        server_instance: "server",
        client_instance: "ime",
        policy_epoch: 4,
        policy_applied: true,
    }
}
fn request() -> TypedCaptureRequest {
    TypedCaptureRequest {
        request_id: "request".into(),
        client_instance: "ime".into(),
        server_instance: "server".into(),
        policy_epoch: 4,
        typed_capture: TypedCaptureCommand::Commit {
            capture_epoch: 2,
            event_id: "event".into(),
            segment_id: "segment".into(),
            text: "已确认的中文片段".into(),
            draft: Box::new(HostTargetToken {
                target_id: "draft".into(),
                host_instance: "ime".into(),
                controller_id: "controller".into(),
                activation_generation: 1,
                field_id: Some("draft".into()),
                selection_generation: 1,
                composition_generation: 1,
                source_app: Some("com.apple.TextEdit".into()),
            }),
        },
    }
}

#[test]
fn typed_wire_is_distinct_and_requires_the_live_authenticated_peer() {
    let request = request();
    assert!(request.validate_for(&peer()).is_ok());
    let encoded = serde_json::to_value(&request).unwrap();
    assert!(matches!(
        serde_json::from_value::<VoiceWireRequest>(encoded.clone()).unwrap(),
        VoiceWireRequest::TypedCapture(_)
    ));
    let mut bad = encoded;
    bad["unexpected"] = true.into();
    assert!(serde_json::from_value::<VoiceWireRequest>(bad).is_err());
    for alter in 0..4 {
        let mut invalid = request.clone();
        match alter {
            0 => invalid.client_instance = "other".into(),
            1 => invalid.server_instance = "restarted".into(),
            2 => invalid.policy_epoch = 3,
            _ => {
                if let TypedCaptureCommand::Commit { draft, .. } = &mut invalid.typed_capture {
                    draft.host_instance = "other".into()
                }
            }
        }
        assert!(invalid.validate_for(&peer()).is_err());
    }
    assert!(request
        .validate_for(&VoicePeer {
            policy_applied: false,
            ..peer()
        })
        .is_err());
}

#[test]
fn unbounded_empty_and_control_character_bodies_are_rejected_before_capture() {
    for body in [
        String::new(),
        "   ".into(),
        "秘密\0正文".into(),
        "中".repeat(2800),
    ] {
        let mut invalid = request();
        if let TypedCaptureCommand::Commit { text, .. } = &mut invalid.typed_capture {
            *text = body;
        }
        assert!(invalid.validate_for(&peer()).is_err());
    }
    let mut policy = request();
    policy.typed_capture = TypedCaptureCommand::Policy;
    assert!(policy.validate_for(&peer()).is_ok());
}
