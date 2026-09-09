import Foundation
import Darwin

private struct OutputRequestFixture: Decodable {
  struct Command: Decodable { let kind: String; let operation_id: String?; let receipt: String? }
  let request_id: String
  let output: Command?
  let menu: Command?
  let command: Command?
}
private struct OutputReplyFixture: Encodable {
  let status: String
  let request_id: String
  var delivery: InputiaVoiceDelivery? = nil
  var operation_id: String? = nil
  var state: String? = nil
  var code: String? = nil
  var view: InputiaVoiceSessionView? = nil
}

private final class SyntheticState: InputiaSharedStateBarrierApplying {
  var applied = 0
  var reject = false
  func applySharedStateBarrier(_ barrier: InputiaVoicePolicyBarrier) throws {
    if reject { throw InputiaVoiceServiceError.policy }
    applied += 1
  }
}

@main
struct InputiaVoiceServiceSelfCheck {
  static func main() throws {
    let shortcutTarget = InputiaVoiceTarget(target_id: "synthetic-target", host_instance: "synthetic-host",
      controller_id: "synthetic-controller", activation_generation: 1, field_id: "synthetic-field",
      selection_generation: 0, composition_generation: 0, source_app: "synthetic.app")
    for activation in [InputiaVoiceShortcutActivation.toggle, .pushToTalk, .holdOrToggle] {
      let edge = InputiaHostShortcutEdge(trigger_id: "synthetic-trigger", starts_session: true,
        lease_id: "synthetic-lease", lease_epoch: 1, binding_id: "transcribe",
        hotkey_string: "alt+space", is_pressed: true, activation: activation,
        pressed_at_unix_ms: 100, hold_threshold_ms: 300)
      let encoded = try JSONEncoder().encode(InputiaVoiceCommand.hostShortcut(target: shortcutTarget,
        postProcess: false, terms: InputiaVoiceTermsVersion(policy_epoch: 3, learning_generation: 7), edge: edge))
      let object = try JSONSerialization.jsonObject(with: encoded) as! [String: Any]
      precondition(object["kind"] as? String == "host_shortcut")
      let encodedEdge = object["edge"] as! [String: Any]
      precondition(encodedEdge["activation"] as? String == activation.rawValue)
      precondition(encodedEdge["starts_session"] as? Bool == true)
      precondition(encodedEdge["lease_epoch"] as? Int == 1)
    }
    print("inputia_host_shortcut_wire=pass modes=3 synthetic_encoding_only=true")
    let state = SyntheticState()
    let good = InputiaVoicePolicyBarrier(barrier_id: String(repeating: "a", count: 64),
      version: InputiaVoiceTermsVersion(policy_epoch: 3, learning_generation: 7), clear_shared_personalization: true)
    var sent = 0
    try InputiaVoiceServiceConnection.checkPolicy(good, minimumEpoch: 3, state: state) {
      precondition(state.applied == 1)
      sent += 1
    }
    precondition(sent == 1)
    state.reject = true
    do {
      try InputiaVoiceServiceConnection.checkPolicy(good, minimumEpoch: 3, state: state) { sent += 1 }
      fatalError("failed local cleanup was acknowledged")
    } catch {}
    precondition(sent == 1)
    state.reject = false
    for invalid in [
      InputiaVoicePolicyBarrier(barrier_id: good.barrier_id, version: good.version, clear_shared_personalization: false),
      InputiaVoicePolicyBarrier(barrier_id: "old", version: good.version, clear_shared_personalization: true),
      InputiaVoicePolicyBarrier(barrier_id: String(repeating: "G", count: 64), version: good.version, clear_shared_personalization: true),
      InputiaVoicePolicyBarrier(barrier_id: good.barrier_id, version: InputiaVoiceTermsVersion(policy_epoch: 2, learning_generation: 9), clear_shared_personalization: true),
    ] {
      do {
        try InputiaVoiceServiceConnection.checkPolicy(invalid, minimumEpoch: 3, state: state) { sent += 1 }
        fatalError("invalid barrier accepted")
      } catch {}
    }
    precondition(state.applied == 1 && sent == 1)
    let raw = Data(#"{"barrier_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","version":{"policy_epoch":3,"learning_generation":7},"clear_shared_personalization":true}"#.utf8)
    let decoded = try JSONDecoder().decode(InputiaVoicePolicyBarrier.self, from: raw)
    precondition(decoded == good)
    for (command, kind) in [(InputiaVoiceCommand.stop, "stop"), (.cancel, "cancel"), (.status, "status")] {
      let encoded = try JSONEncoder().encode(command)
      let object = try JSONSerialization.jsonObject(with: encoded) as! [String: String]
      precondition(object == ["kind": kind])
    }
    let rejected = try JSONDecoder().decode(InputiaVoiceReply.self,
      from: Data(#"{"status":"rejected","request_id":"request-1","code":"unknown"}"#.utf8))
    precondition(rejected.code == "unknown" && rejected.view == nil && rejected.request_id == "request-1")
    let deliveryObject: [String: Any] = ["operation_id": "op", "session_id": "session", "item_id": "item",
      "revision": 1, "policy_epoch": 3, "target_id": "target", "text": "合成测试"]
    let deliveryView = InputiaVoiceSessionView(session_id: "session", generation: 1,
      phase: "pending_target", target_id: "target", item_id: "item", output_operation_id: "op")
    let deliveryTarget = InputiaVoiceTarget(target_id: "target", host_instance: "host", controller_id: "controller",
      activation_generation: 1, field_id: "field", selection_generation: 0, composition_generation: 0, source_app: "test")
    func decodeDelivery(_ object: [String: Any]) throws -> InputiaVoiceDelivery {
      try JSONDecoder().decode(InputiaVoiceDelivery.self, from: JSONSerialization.data(withJSONObject: object))
    }
    let delivery = try decodeDelivery(deliveryObject)
    precondition(delivery.dispatchDeadline == 0)
    try InputiaVoiceServiceConnection.validateDelivery(delivery, view: deliveryView, target: deliveryTarget, epoch: 3)
    for (key, value): (String, Any) in [("operation_id", "other"), ("session_id", "other"),
      ("item_id", "other"), ("target_id", "other"), ("policy_epoch", 2), ("text", ""),
      ("text", String(repeating: "中", count: 65537))] {
      var changed = deliveryObject
      changed[key] = value
      do {
        try InputiaVoiceServiceConnection.validateDelivery(decodeDelivery(changed), view: deliveryView, target: deliveryTarget, epoch: 3)
        fatalError("mismatched delivery accepted")
      } catch {}
    }
    print("inputia_voice_delivery_contract=pass mismatched_identity_epoch_and_payload_rejected=true native_insertion_tested=false")
    let finished = DispatchSemaphore(value: 0)
    DispatchQueue.global().async {
      do {
        var descriptors: [Int32] = [0, 0]
        precondition(socketpair(AF_UNIX, SOCK_STREAM, 0, &descriptors) == 0)
        let client = try InputiaVoiceServiceConnection.fixture(descriptor: descriptors[0],
          server: InputiaVoiceHello(protocol_major: 1, protocol_minor: 0, instance_id: "server",
            profile_id: "synthetic", policy_epoch: 3, capabilities: ["voice_sessions_v1"]), version: good.version)
        let peerFD = descriptors[1]
        let peerFinished = DispatchSemaphore(value: 0)
        DispatchQueue.global().async {
          do {
            let peer = try InputiaFramedConnection.fixture(descriptor: peerFD, timeout: 2)
            let fetch = try peer.read(OutputRequestFixture.self)
            precondition(fetch.output?.kind == "fetch")
            try peer.write(OutputReplyFixture(status: "delivery", request_id: fetch.request_id, delivery: delivery))
            let receipt = try peer.read(OutputRequestFixture.self)
            precondition(receipt.output?.kind == "receipt" && receipt.output?.operation_id == "op" && receipt.output?.receipt == "dispatched")
            try peer.write(OutputReplyFixture(status: "output", request_id: receipt.request_id, operation_id: "op", state: "dispatched_only"))
            let navigation = try peer.read(OutputRequestFixture.self)
            precondition(navigation.menu?.kind == "history")
            try peer.write(OutputReplyFixture(status: "rejected", request_id: navigation.request_id, code: "coordinator_rejected"))
            let query = try peer.read(OutputRequestFixture.self)
            precondition(query.command?.kind == "status")
            try peer.write(OutputReplyFixture(status: "session", request_id: query.request_id, view: deliveryView))
            peer.close()
          } catch { fatalError("synthetic delivery peer failed: \(error)") }
          peerFinished.signal()
        }
        guard let received = try client.fetchDelivery(view: deliveryView, target: deliveryTarget) else { fatalError("delivery missing") }
        precondition(received.dispatchDeadline > ProcessInfo.processInfo.systemUptime)
        try client.acknowledgeDelivery(received, receipt: "dispatched")
        let declinedMenu = try client.menuRequest(kind: "history")
        precondition(declinedMenu.status == "rejected")
        let stillConnected = try client.request(sessionID: "session", requestID: "status-after-menu", command: .status)
        precondition(stillConnected.view?.session_id == "session")
        client.close()
        precondition(peerFinished.wait(timeout: .now() + 3) == .success)
        print("inputia_voice_output_wire=pass fetch_and_receipt=true synthetic_peer=true native_insertion_tested=false")
      } catch { fatalError("synthetic delivery client failed: \(error)") }
      finished.signal()
    }
    precondition(finished.wait(timeout: .now() + 5) == .success)
    print("inputia_voice_policy_client=pass checks=7 cleanup_before_ack=true failed_cleanup_no_ack=true synthetic_state_only=true actual_host_cache_not_tested=true")
  }
}
