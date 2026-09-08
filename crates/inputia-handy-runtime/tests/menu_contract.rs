#[cfg(test)]
mod menu_contract_tests {
    use inputia_handy_runtime::voice_protocol::*;

    #[test]
    fn recording_keeps_navigation_available_but_blocks_engine_changes() {
        for command in [MenuCommand::Status, MenuCommand::History, MenuCommand::Settings,
            MenuCommand::CheckUpdates, MenuCommand::CopyLatest] {
            assert!(command.allowed_while_busy());
        }
        for command in [MenuCommand::UnloadModel, MenuCommand::QuitService,
            MenuCommand::SelectModel { model_id: "local".into() }] {
            assert!(!command.allowed_while_busy());
        }
    }

    #[test]
    fn menu_wire_is_distinct_and_rejects_extra_payloads() {
        let wire = serde_json::json!({
            "request_id":"request", "client_instance":"client", "server_instance":"server",
            "policy_epoch":7, "menu":{"kind":"select_model","model_id":"local-model"}
        });
        assert!(matches!(
            serde_json::from_value::<VoiceWireRequest>(wire.clone()),
            Ok(VoiceWireRequest::Menu(_))
        ));
        let mut invalid = wire;
        invalid["menu"]["path"] = serde_json::json!("/tmp/arbitrary");
        assert!(serde_json::from_value::<VoiceWireRequest>(invalid).is_err());
    }

    #[test]
    fn menu_requires_current_policy_and_exact_connection_identity() {
        let request = MenuRequest {
            request_id: "request".into(),
            client_instance: "client".into(),
            server_instance: "server".into(),
            policy_epoch: 7,
            menu: MenuCommand::Status,
        };
        let mut peer = VoicePeer {
            client_instance: "client",
            server_instance: "server",
            policy_epoch: 7,
            policy_applied: true,
        };
        assert!(request.validate_for(&peer).is_ok());
        peer.policy_applied = false;
        assert!(request.validate_for(&peer).is_err());
        peer.policy_applied = true;
        peer.policy_epoch = 8;
        assert!(request.validate_for(&peer).is_err());
        peer.policy_epoch = 7;
        peer.client_instance = "other";
        assert!(request.validate_for(&peer).is_err());
        peer.client_instance = "client";
        peer.server_instance = "restarted";
        assert!(request.validate_for(&peer).is_err());
    }
}
