import Foundation

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
    print("inputia_voice_policy_client=pass checks=7 cleanup_before_ack=true failed_cleanup_no_ack=true synthetic_state_only=true actual_host_cache_not_tested=true")
  }
}
