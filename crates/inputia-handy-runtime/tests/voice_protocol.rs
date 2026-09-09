use inputia_handy_runtime::voice_protocol::*;

#[test]
fn control_reply_is_bound_to_request_and_session_without_exposing_error_details() {
    let request = request();
    let view = VoiceSessionView {
        session_id: request.session_id.clone(),
        generation: 1,
        phase: VoicePhase::Preparing,
        target_id: Some("target-1".into()),
        item_id: None,
        output_operation_id: None,
    };
    let reply = VoiceReply::Session {
        request_id: request.request_id.clone(),
        view: view.clone(),
    };
    reply.validate_for(&request).unwrap();
    let mut other = request.clone();
    other.session_id = "another".into();
    assert!(reply.validate_for(&other).is_err());
    other = request.clone();
    other.request_id = "another-request".into();
    assert!(reply.validate_for(&other).is_err());
    let error = VoiceReply::Rejected {
        request_id: request.request_id.clone(),
        code: VoiceReplyError::Unknown,
    };
    error.validate_for(&request).unwrap();
    assert_eq!(
        serde_json::to_string(&error).unwrap(),
        r#"{"status":"rejected","request_id":"request-1","code":"unknown"}"#
    );
    let restored: VoiceReply =
        serde_json::from_str(&serde_json::to_string(&reply).unwrap()).unwrap();
    assert_eq!(restored, reply);
}

#[test]
fn policy_barrier_requires_current_version_and_both_cleanup_receipts() {
    let version = VoiceTermsVersion {
        policy_epoch: 7,
        learning_generation: 11,
    };
    let barrier = VoicePolicyBarrier::new(version.clone()).unwrap();
    let ack = VoicePolicyAcknowledgement {
        barrier_id: barrier.barrier_id.clone(),
        version: version.clone(),
        shared_cache_cleared: true,
        offline_queue_revalidated: true,
    };
    assert!(barrier.validate_ack(&ack, &version).is_ok());
    for kind in 0..5 {
        let mut invalid = ack.clone();
        match kind {
            0 => invalid.barrier_id = "old-connection".into(),
            1 => invalid.version.policy_epoch += 1,
            2 => invalid.version.learning_generation += 1,
            3 => invalid.shared_cache_cleared = false,
            _ => invalid.offline_queue_revalidated = false,
        }
        assert!(barrier.validate_ack(&invalid, &version).is_err());
    }
    for current in [
        VoiceTermsVersion {
            policy_epoch: 8,
            learning_generation: 11,
        },
        VoiceTermsVersion {
            policy_epoch: 7,
            learning_generation: 12,
        },
    ] {
        assert!(barrier.validate_ack(&ack, &current).is_err());
    }
}

#[test]
fn new_policy_barrier_never_accepts_previous_connection_ack() {
    let version = VoiceTermsVersion {
        policy_epoch: 1,
        learning_generation: 0,
    };
    let first = VoicePolicyBarrier::new(version.clone()).unwrap();
    let second = VoicePolicyBarrier::new(version.clone()).unwrap();
    assert_ne!(first.barrier_id, second.barrier_id);
    let ack = VoicePolicyAcknowledgement {
        barrier_id: first.barrier_id,
        version: version.clone(),
        shared_cache_cleared: true,
        offline_queue_revalidated: true,
    };
    assert!(second.validate_ack(&ack, &version).is_err());
    let raw = serde_json::to_string(&second).unwrap();
    assert!(!raw.contains("terms\":"));
    assert!(serde_json::from_str::<VoicePolicyAcknowledgement>(r#"{"barrier_id":"x","version":{"policy_epoch":1,"learning_generation":0},"shared_cache_cleared":true}"#).is_err());
}

fn request() -> VoiceRequest {
    VoiceRequest {
        request_id: "request-1".into(),
        session_id: "session-1".into(),
        server_instance: "server-1".into(),
        client_instance: "host-1".into(),
        policy_epoch: 7,
        command: VoiceCommand::Start {
            target: HostTargetToken {
                target_id: "target-1".into(),
                host_instance: "host-1".into(),
                controller_id: "controller-1".into(),
                activation_generation: 2,
                field_id: Some("field-1".into()),
                selection_generation: 3,
                composition_generation: 4,
                source_app: Some("com.example.fixture".into()),
            },
            post_process: false,
            terms: VoiceTermsVersion {
                policy_epoch: 7,
                learning_generation: 11,
            },
        },
    }
}
fn peer() -> VoicePeer<'static> {
    VoicePeer {
        server_instance: "server-1",
        client_instance: "host-1",
        policy_epoch: 7,
        policy_applied: true,
    }
}

#[test]
fn wire_shape_is_explicit_and_rejects_arbitrary_fields() {
    let wire = serde_json::to_value(request()).unwrap();
    assert_eq!(wire["command"]["kind"], "start");
    assert_eq!(wire["command"]["terms"]["learning_generation"], 11);
    assert_eq!(
        serde_json::from_value::<VoiceRequest>(wire.clone()).unwrap(),
        request()
    );
    let mut extra = wire.clone();
    extra["command"]["shell"] = "not permitted".into();
    assert!(serde_json::from_value::<VoiceRequest>(extra).is_err());
    let mut extra = wire;
    extra["command"]["target"]["path"] = "/arbitrary".into();
    assert!(serde_json::from_value::<VoiceRequest>(extra).is_err());
}

#[test]
fn authenticated_connection_and_current_policy_are_required_for_start() {
    let mut request = request();
    assert!(request.validate_for(&peer()).is_ok());
    request.server_instance = "server-before-restart".into();
    assert!(request.validate_for(&peer()).is_err());
    request.server_instance = "server-1".into();
    request.client_instance = "another-host".into();
    assert!(request.validate_for(&peer()).is_err());
    request.client_instance = "host-1".into();
    request.policy_epoch = 6;
    assert!(request.validate_for(&peer()).is_err());
    request.policy_epoch = 7;
    assert!(request
        .validate_for(&VoicePeer {
            policy_applied: false,
            ..peer()
        })
        .is_err());
    if let VoiceCommand::Start { target, .. } = &mut request.command {
        target.host_instance = "other-host".into();
    }
    assert!(request.validate_for(&peer()).is_err());
}

#[test]
fn withdrawal_does_not_prevent_stop_or_cancel_but_future_epochs_are_rejected() {
    for command in [
        VoiceCommand::Stop,
        VoiceCommand::Cancel,
        VoiceCommand::Status,
    ] {
        let request = VoiceRequest {
            command,
            policy_epoch: 6,
            ..request()
        };
        assert!(request
            .validate_for(&VoicePeer {
                policy_applied: false,
                ..peer()
            })
            .is_ok());
        assert!(VoiceRequest {
            policy_epoch: 8,
            ..request
        }
        .validate_for(&peer())
        .is_err());
    }
}

#[test]
fn host_shortcut_start_requires_policy_but_later_edges_can_close_old_session() {
    let mut start = request();
    let VoiceCommand::Start {
        target,
        post_process,
        terms,
    } = start.command
    else {
        unreachable!();
    };
    start.command = VoiceCommand::HostShortcut {
        target: target.clone(),
        post_process,
        terms: terms.clone(),
        edge: HostShortcutEdge {
            trigger_id: "trigger-1".into(),
            starts_session: true,
            lease_id: "lease-1".into(),
            lease_epoch: 1,
            binding_id: "transcribe".into(),
            hotkey_string: "Option+Space".into(),
            is_pressed: true,
            activation: VoiceShortcutActivation::PushToTalk,
            pressed_at_unix_ms: 1,
            hold_threshold_ms: 0,
        },
    };
    assert!(start.validate_for(&peer()).is_ok());
    assert!(start
        .validate_for(&VoicePeer {
            policy_applied: false,
            ..peer()
        })
        .is_err());

    let mut release = start.clone();
    release.request_id = "request-2".into();
    release.policy_epoch = 6;
    release.command = VoiceCommand::HostShortcut {
        target,
        post_process,
        terms,
        edge: HostShortcutEdge {
            trigger_id: "trigger-2".into(),
            starts_session: false,
            lease_id: "lease-1".into(),
            lease_epoch: 1,
            binding_id: "transcribe".into(),
            hotkey_string: "Option+Space".into(),
            is_pressed: false,
            activation: VoiceShortcutActivation::PushToTalk,
            pressed_at_unix_ms: 1,
            hold_threshold_ms: 0,
        },
    };
    assert!(release
        .validate_for(&VoicePeer {
            policy_applied: false,
            ..peer()
        })
        .is_ok());
}

#[test]
fn missing_field_identity_is_representable_but_never_manufactured_from_bundle_id() {
    let mut request = request();
    if let VoiceCommand::Start { target, .. } = &mut request.command {
        target.field_id = None;
    }
    assert!(request.validate_for(&peer()).is_ok());
    let wire = serde_json::to_value(request).unwrap();
    assert!(wire["command"]["target"]["field_id"].is_null());
}

#[test]
fn delivery_keeps_body_off_debug_and_bounds_wire_without_truncating_text() {
    let mut delivery = VoiceDelivery {
        operation_id: "output-1".into(),
        session_id: "session-1".into(),
        item_id: "voice-store:record-1".into(),
        revision: 1,
        policy_epoch: 7,
        target_id: "target-1".into(),
        text: "合成正文\n第二行".into(),
    };
    assert!(delivery.validate().is_ok());
    let decoded: VoiceDelivery =
        serde_json::from_str(&serde_json::to_string(&delivery).unwrap()).unwrap();
    assert!(decoded == delivery);
    delivery.text = "x".repeat(MAX_DELIVERY_TEXT_BYTES + 1);
    assert!(delivery.validate().is_err());
    assert_eq!(delivery.text.len(), MAX_DELIVERY_TEXT_BYTES + 1);
}

#[test]
fn wire_accepts_control_or_output_frames_without_mixing_contracts() {
    let control = serde_json::to_value(request()).unwrap();
    assert!(matches!(
        serde_json::from_value::<VoiceWireRequest>(control).unwrap(),
        VoiceWireRequest::Control(_)
    ));
    let fetch = serde_json::json!({
        "request_id": "fetch-1",
        "session_id": "session-1",
        "server_instance": "server-1",
        "client_instance": "host-1",
        "policy_epoch": 7,
        "output": {"kind": "fetch"}
    });
    let VoiceWireRequest::Output(output) =
        serde_json::from_value::<VoiceWireRequest>(fetch).unwrap()
    else {
        panic!("expected output request")
    };
    output.validate_for(&peer()).unwrap();
    assert!(matches!(output.output, VoiceOutputCommand::Fetch {}));
    let mut extra = serde_json::to_value(output).unwrap();
    extra["output"]["debug_text"] = "正文不能混入请求".into();
    assert!(serde_json::from_value::<VoiceOutputRequest>(extra).is_err());
}

#[test]
fn output_reply_shapes_match_swift_decoder_expectations() {
    let delivery = VoiceDelivery {
        operation_id: "output-1".into(),
        session_id: "session-1".into(),
        item_id: "voice-store:record-1".into(),
        revision: 1,
        policy_epoch: 7,
        target_id: "target-1".into(),
        text: "合成正文".into(),
    };
    let request = VoiceOutputRequest {
        request_id: "fetch-1".into(),
        session_id: "session-1".into(),
        server_instance: "server-1".into(),
        client_instance: "host-1".into(),
        policy_epoch: 7,
        output: VoiceOutputCommand::Fetch {},
    };
    let reply = VoiceOutputReply::Delivery {
        request_id: "fetch-1".into(),
        delivery,
    };
    reply.validate_for(&request).unwrap();
    let raw = serde_json::to_value(&reply).unwrap();
    assert_eq!(raw["status"], "delivery");
    assert!(raw.get("state").is_none());
    let state_reply = VoiceOutputReply::Output {
        request_id: "receipt-1".into(),
        operation_id: "output-1".into(),
        state: inputia_handy_runtime::output_ledger::OutputState::DispatchedOnly,
    };
    let raw = serde_json::to_value(state_reply).unwrap();
    assert_eq!(raw["status"], "output");
    assert_eq!(raw["state"], "dispatched_only");
    assert!(raw.get("delivery").is_none());
}
