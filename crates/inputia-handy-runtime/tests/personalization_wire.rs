use inputia_handy_runtime::{
    personalization_wire::{PersonalizationCommand, PersonalizationRequest},
    voice_protocol::{HostTargetToken, VoicePeer, VoiceWireRequest},
};
use serde_json::json;

fn request() -> PersonalizationRequest {
    PersonalizationRequest {
        request_id: "r".into(),
        client_instance: "ime".into(),
        server_instance: "server".into(),
        policy_epoch: 1,
        personalization: PersonalizationCommand::Query {
            target: Box::new(HostTargetToken {
                target_id: "original-field".into(),
                host_instance: "ime".into(),
                controller_id: "controller".into(),
                activation_generation: 1,
                field_id: Some("original-field".into()),
                selection_generation: 1,
                composition_generation: 1,
                source_app: Some("com.apple.TextEdit".into()),
            }),
            learning_epoch: 1,
            input_code: "ba".into(),
            schema_id: String::new(),
            context: "对".into(),
            context_id: "original-field".into(),
            candidates: vec![json!({"id":"rime:x:0","text":"吧","base_rank":0,"consumed_len":2})],
            limit: 3,
        },
    }
}
fn peer() -> VoicePeer<'static> {
    VoicePeer {
        server_instance: "server",
        client_instance: "ime",
        policy_epoch: 1,
        policy_applied: true,
    }
}

#[test]
fn personalization_requires_the_live_peer_and_original_field_identity() {
    let r = request();
    assert!(r.validate_for(&peer()).is_ok());
    assert!(matches!(
        serde_json::from_value::<VoiceWireRequest>(serde_json::to_value(&r).unwrap()).unwrap(),
        VoiceWireRequest::Personalization(_)
    ));
    for mutation in 0..5 {
        let mut r = request();
        match mutation {
            0 => r.client_instance = "other".into(),
            1 => r.server_instance = "restarted".into(),
            2 => r.policy_epoch = 2,
            3 => {
                if let PersonalizationCommand::Query { context_id, .. } = &mut r.personalization {
                    *context_id = "other-field".into()
                }
            }
            _ => {
                if let PersonalizationCommand::Query { target, .. } = &mut r.personalization {
                    target.field_id = None
                }
            }
        }
        assert!(r.validate_for(&peer()).is_err());
    }
    assert!(r
        .validate_for(&VoicePeer {
            policy_applied: false,
            ..peer()
        })
        .is_err());
}

#[test]
fn prediction_admission_carries_current_learning_epoch_and_exact_candidate() {
    let mut r = request();
    let PersonalizationCommand::Query { target, .. } = r.personalization else {
        unreachable!()
    };
    r.personalization = PersonalizationCommand::Admit {
        target,
        learning_epoch: 7,
        context_id: "original-field".into(),
        context: "我觉得".into(),
        text: "可以".into(),
        prediction_id: "prediction-1".into(),
        input_code: String::new(),
        schema_id: String::new(),
    };
    assert!(r.validate_for(&peer()).is_ok());
    if let PersonalizationCommand::Admit { prediction_id, .. } = &mut r.personalization {
        *prediction_id = String::new()
    }
    assert!(r.validate_for(&peer()).is_err());
}

#[test]
fn recall_admission_requires_schema_and_ascii_code_while_legacy_query_defaults_work() {
    let mut legacy = serde_json::to_value(request()).unwrap();
    legacy["personalization"]
        .as_object_mut()
        .unwrap()
        .remove("schema_id");
    let decoded: PersonalizationRequest = serde_json::from_value(legacy).unwrap();
    assert!(decoded.validate_for(&peer()).is_ok());
    let mut r = request();
    let PersonalizationCommand::Query { target, .. } = r.personalization else {
        unreachable!()
    };
    r.personalization = PersonalizationCommand::Admit {
        target,
        learning_epoch: 1,
        context_id: "original-field".into(),
        context: "语境".into(),
        text: "已学词".into(),
        prediction_id: "learned:stable-id".into(),
        schema_id: "double_pinyin_flypy".into(),
        input_code: "abcd".into(),
    };
    assert!(r.validate_for(&peer()).is_ok());
    let mut no_schema = r.clone();
    if let PersonalizationCommand::Admit { schema_id, .. } = &mut no_schema.personalization {
        schema_id.clear();
    }
    assert!(no_schema.validate_for(&peer()).is_err());
    if let PersonalizationCommand::Admit { input_code, .. } = &mut r.personalization {
        *input_code = "拼音".into();
    }
    assert!(r.validate_for(&peer()).is_err());
}
