use inputia_handy_runtime::voice_protocol::*;

#[test]
fn shared_terms_wire_preserves_explicit_hotword_priority_and_lease_identity() {
    use inputia_core::integration::terms::{build_hotwords, HotwordBudget, TermEvidence};
    let custom_words = vec!["中古".into(), "把".into()];
    let terms = build_hotwords(
        &custom_words,
        &[("学习词".into(), TermEvidence::ConfirmedCorrection)],
        HotwordBudget::default(),
    )
    .unwrap();
    let reply = SharedTermsReply::SharedTerms {
        request_id: "hotword-wire".into(),
        lease_id: "lease-1".into(),
        lease_epoch: 2,
        version: VoiceTermsVersion {
            policy_epoch: 7,
            learning_generation: 3,
        },
        terms,
        max_age_ms: 500,
    };
    let bytes = serde_json::to_vec(&reply).unwrap();
    let decoded: SharedTermsReply = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(decoded, reply);
    let SharedTermsReply::SharedTerms {
        terms, lease_epoch, ..
    } = decoded
    else {
        panic!("terms rejected")
    };
    assert_eq!(&terms[..2], &custom_words);
    assert_eq!(lease_epoch, 2);
}

#[test]
fn shared_terms_wire_binds_authenticated_peer_and_only_accepts_lease_identity() {
    let value = serde_json::json!({"request_id":"terms-1","client_instance":"host-1","server_instance":"server-1","policy_epoch":7,"shared_terms":{"lease_id":"lease-1","lease_epoch":2}});
    let VoiceWireRequest::SharedTerms(request) = serde_json::from_value(value.clone()).unwrap()
    else {
        panic!("wrong wire variant");
    };
    request.validate_for(&peer()).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), value);
    for altered in [
        VoicePeer {
            client_instance: "other",
            ..peer()
        },
        VoicePeer {
            server_instance: "other",
            ..peer()
        },
        VoicePeer {
            policy_epoch: 8,
            ..peer()
        },
        VoicePeer {
            policy_applied: false,
            ..peer()
        },
    ] {
        assert!(request.validate_for(&altered).is_err());
    }
    let mut bad = value;
    bad["shared_terms"]["target"] = serde_json::json!({"source_app":"fake"});
    assert!(serde_json::from_value::<VoiceWireRequest>(bad).is_err());
    let mut bad = request;
    bad.shared_terms.lease_epoch = 0;
    assert!(bad.validate_for(&peer()).is_err());
    let reply = serde_json::json!({"status":"shared_terms","request_id":"terms-1","lease_id":"lease-1","lease_epoch":2,"version":{"policy_epoch":7,"learning_generation":3},"terms":["Inputia"],"max_age_ms":500});
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<SharedTermsReply>(reply.clone()).unwrap())
            .unwrap(),
        reply
    );
    let rejected =
        serde_json::json!({"status":"rejected","request_id":"terms-1","code":"unauthorized"});
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<SharedTermsReply>(rejected.clone()).unwrap())
            .unwrap(),
        rejected
    );
}

#[test]
fn shortcut_trigger_reply_keeps_existing_wire_shape() {
    let value = serde_json::json!({
        "status":"trigger", "request_id":"poll-1", "trigger": {
            "trigger_id":"edge-1", "session_id":"session-1", "starts_session":true,
            "lease_id":"lease-1", "lease_epoch":1,
            "target": {"target_id":"field-token", "host_instance":"host-1", "controller_id":"controller-1",
                "activation_generation":1,"field_id":"field-1","selection_generation":1,
                "composition_generation":1,"source_app":"synthetic.editor"},
            "binding_id":"transcribe","hotkey_string":"Option+Space", "is_pressed":true,
            "activation":"toggle","pressed_at_unix_ms":1,"hold_threshold_ms":400,
            "server_instance":"server-1","client_instance":"host-1","policy_epoch":1
        }
    });
    let decoded: HostShortcutReply = serde_json::from_value(value.clone()).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), value);
}

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

#[test]
fn target_bridge_requires_authenticated_envelope_and_operation_bound_dispatch() {
    let value = serde_json::json!({
        "request_id":"target-1", "client_instance":"host-1", "server_instance":"server-1", "policy_epoch":7,
        "target_bridge":{"kind":"capture", "draft":{
            "target_id":"draft", "host_instance":"host-1", "controller_id":"controller-1", "activation_generation":1,
            "field_id":null, "selection_generation":2, "composition_generation":3, "source_app":"synthetic.editor"
        }}
    });
    let VoiceWireRequest::TargetBridge(request) = serde_json::from_value(value.clone()).unwrap()
    else {
        panic!("wrong variant")
    };
    request.validate_for(&peer()).unwrap();
    for bad_peer in [
        VoicePeer {
            client_instance: "other",
            ..peer()
        },
        VoicePeer {
            server_instance: "other",
            ..peer()
        },
        VoicePeer {
            policy_epoch: 8,
            ..peer()
        },
        VoicePeer {
            policy_applied: false,
            ..peer()
        },
    ] {
        assert!(request.validate_for(&bad_peer).is_err());
    }
    let TargetBridgeCommand::Capture { draft } = request.target_bridge.clone() else {
        unreachable!()
    };
    let mut validation = request;
    validation.target_bridge = TargetBridgeCommand::Validate {
        target: draft.clone(),
        purpose: TargetBridgePurpose::Dispatch,
        operation_id: None,
    };
    assert!(validation.validate_for(&peer()).is_err());
    validation.target_bridge = TargetBridgeCommand::Validate {
        target: draft,
        purpose: TargetBridgePurpose::Dispatch,
        operation_id: Some("operation".into()),
    };
    validation.validate_for(&peer()).unwrap();
    let mut injected = value;
    injected["target_bridge"]["pid"] = serde_json::json!(42);
    assert!(serde_json::from_value::<VoiceWireRequest>(injected).is_err());
}
