import Foundation
import Darwin

private struct Request: Decodable {
  struct Lease: Decodable { let lease_id: String; let lease_epoch: UInt64 }
  let request_id: String
  let client_instance: String
  let server_instance: String
  let policy_epoch: UInt64
  let shared_terms: Lease
}
private struct Reply: Encodable {
  let status = "shared_terms"
  let request_id: String
  let lease_id: String
  let lease_epoch: UInt64
  let version: InputiaVoiceTermsVersion
  let terms = ["Inputia"]
  let max_age_ms: UInt64
}
private final class ClearedState: InputiaSharedStateBarrierApplying {
  func applySharedStateBarrier(_ barrier: InputiaVoicePolicyBarrier) throws {
    precondition(InputiaSharedTermsMemory.shared.current(target: InputiaSharedTermsSelfCheck.target()) == nil)
  }
}

@main struct InputiaSharedTermsSelfCheck {
  static func target(_ id: String = "target") -> InputiaVoiceTarget {
    InputiaVoiceTarget(target_id: id, host_instance: InputiaVoiceServiceConnection.processInstance,
      controller_id: "controller", activation_generation: 1, field_id: "field",
      selection_generation: 1, composition_generation: 1, source_app: "synthetic.test")
  }
  static func main() {
    let words = ["lll@example.com", "compute服务器", "AEA", "额度", "南信大", "绿洲"]
    func hotwords(_ code: String) -> [String] { InputiaHotwordPrefix.candidates(words, code: code) }
    precondition(hotwords("lll") == ["lll@example.com"])
    precondition(hotwords("llll").isEmpty && hotwords("lllll").isEmpty)
    precondition(hotwords(String("llll".dropLast())) == ["lll@example.com"])
    precondition(hotwords("com") == ["compute服务器"] && hotwords("comp").isEmpty)
    precondition(hotwords("aea") == ["AEA"])
    precondition(hotwords("edu") == ["额度"] && hotwords("eedu") == ["额度"])
    precondition(hotwords("nanxin") == ["南信大"] && hotwords("njxn") == ["南信大"])
    precondition(hotwords("nanxinda").isEmpty && hotwords("njxnda").isEmpty)
    precondition(hotwords("lvzhou") == ["绿洲"] && hotwords("lvvb") == ["绿洲"])
    precondition(InputiaHotwordPrefix.candidates(words, code: "eedu", naturalDoublePinyin: false).isEmpty)
    precondition(InputiaHotwordPrefix.candidates([], code: "lll").isEmpty)
    let explicitSnapshot = InputiaSharedTermsSnapshot(identity: "explicit", target: target(),
      version: .init(policy_epoch: 1, learning_generation: 1), terms: ["compute服务器", "composer", "lll@example.com"],
      explicitTerms: words, expiresAt: 10)
    precondition(explicitSnapshot.englishCandidates(prefix: "lll") == ["lll@example.com"])
    precondition(explicitSnapshot.englishCandidates(prefix: "llll").isEmpty)
    precondition(explicitSnapshot.englishCandidates(prefix: "com") == ["compute服务器", "composer"])
    precondition(explicitSnapshot.englishCandidates(prefix: "comp") == ["composer"])
    let orderJSON = Data(#"{"ok":true,"mode":"Chinese","composing":"nihao","page":0,"indices":[1,0,2]}"#.utf8)
    let order = InputiaSharedCandidateOrder.decode(orderJSON, mode: "Chinese", composing: "nihao", page: 0, count: 3)!
    precondition(order.originalIndex(displayed: 0) == 1 && order.originalIndex(displayed: 1) == 0)
    precondition(order.originalIndex(displayed: 3) == nil && order.originalIndex(displayed: -1) == nil)
    for indices in ["[1,1,2]", "[0,1]", "[-1,0,1]", "[0,1,3]", "[true,0,2]"] {
      let json = Data("{\"ok\":true,\"mode\":\"Chinese\",\"composing\":\"nihao\",\"page\":0,\"indices\":\(indices)}".utf8)
      precondition(InputiaSharedCandidateOrder.decode(json, mode: "Chinese", composing: "nihao", page: 0, count: 3) == nil)
    }
    precondition(InputiaSharedCandidateOrder.decode(orderJSON, mode: "English", composing: "nihao", page: 0, count: 3) == nil)
    precondition(InputiaSharedCandidateOrder.decode(orderJSON, mode: "Chinese", composing: "ni", page: 0, count: 3) == nil)
    precondition(InputiaSharedCandidateOrder.decode(orderJSON, mode: "Chinese", composing: "nihao", page: 1, count: 3) == nil)
    let originalCandidates = ["甲", "乙", "丙"]
    precondition(order.matches(mode: "Chinese", composing: "nihao", page: 0, candidates: originalCandidates, originalCandidates: originalCandidates))
    precondition(!order.matches(mode: "Chinese", composing: "nihao", page: 0, candidates: ["新", "乙", "丙"], originalCandidates: originalCandidates))
    precondition(!order.matches(mode: "Chinese", composing: "ni", page: 0, candidates: originalCandidates, originalCandidates: originalCandidates))
    precondition(!order.matches(mode: "Chinese", composing: "nihao", page: 1, candidates: originalCandidates, originalCandidates: originalCandidates))
    let client = NSObject()
    let otherClient = NSObject()
    typealias IntentContext = InputiaSharedEnglishSelectionState.Context
    let original = IntentContext(prefix: "In", targetID: "target", cacheIdentity: "cache",
      clientIdentity: ObjectIdentifier(client), activation: 1)
    var intentState = InputiaSharedEnglishSelectionState()
    let first = intentState.begin(original)!
    precondition(intentState.begin(original) == nil)
    precondition(intentState.consume(first, context: original, gateAllowed: true))
    precondition(!intentState.consume(first, context: original, gateAllowed: true))
    for changed in [
      IntentContext(prefix: "Inp", targetID: "target", cacheIdentity: "cache", clientIdentity: ObjectIdentifier(client), activation: 1),
      IntentContext(prefix: "In", targetID: "other", cacheIdentity: "cache", clientIdentity: ObjectIdentifier(client), activation: 1),
      IntentContext(prefix: "In", targetID: "target", cacheIdentity: "new-cache", clientIdentity: ObjectIdentifier(client), activation: 1),
      IntentContext(prefix: "In", targetID: "target", cacheIdentity: "cache", clientIdentity: ObjectIdentifier(otherClient), activation: 1),
      IntentContext(prefix: "In", targetID: "target", cacheIdentity: "cache", clientIdentity: ObjectIdentifier(client), activation: 2),
    ] {
      let id = intentState.begin(original)!
      precondition(!intentState.consume(id, context: changed, gateAllowed: true))
      precondition(!intentState.hasPending)
    }
    let failedGate = intentState.begin(original)!
    precondition(!intentState.consume(failedGate, context: original, gateAllowed: false))
    let cancelled = intentState.begin(original)!
    intentState.cancel()
    precondition(!intentState.consume(cancelled, context: original, gateAllowed: true))
    let memory = InputiaSharedTermsMemory()
    let version = InputiaVoiceTermsVersion(policy_epoch: 3, learning_generation: 7)
    let entry = InputiaSharedTermsSnapshot(identity: "request", target: target(), version: version,
      terms: ["Inputia"], expiresAt: 12)
    let ticket = memory.ticket()
    precondition(memory.install(entry, ticket: ticket, now: 10))
    precondition(memory.current(target: target(), now: 11)?.identity == "request")
    precondition(memory.current(target: target("different"), now: 11) == nil)
    precondition(memory.current(target: target(), now: 12) == nil)
    var cleared = false
    memory.didClear = { cleared = memory.current(target: target(), now: 10) == nil }
    memory.clear()
    precondition(cleared && !memory.install(entry, ticket: ticket, now: 10))
    precondition(entry.englishCandidates(prefix: "in") == ["Inputia"])
    precondition(entry.englishCandidates(prefix: "Inputia").isEmpty)
    precondition(entry.englishCandidates(prefix: "other").isEmpty)
    let nextVersion = InputiaVoiceTermsVersion(policy_epoch: 3, learning_generation: 8)
    precondition(InputiaVoiceServiceConnection.sharedTermsConnectionMatches(server: "server", primaryServer: "server", version: nextVersion, primaryVersion: version))
    precondition(!InputiaVoiceServiceConnection.sharedTermsConnectionMatches(server: "other", primaryServer: "server", version: nextVersion, primaryVersion: version))
    precondition(!InputiaVoiceServiceConnection.sharedTermsConnectionMatches(server: "server", primaryServer: "server", version: InputiaVoiceTermsVersion(policy_epoch: 4, learning_generation: 8), primaryVersion: version))
    let global = InputiaSharedTermsMemory.shared
    let barrierEntry = InputiaSharedTermsSnapshot(identity: "before-barrier", target: target(), version: version, terms: ["Inputia"], expiresAt: ProcessInfo.processInfo.systemUptime + 2)
    precondition(global.install(barrierEntry, ticket: global.ticket()))
    var uiCleared = false
    global.didClear = { uiCleared = true }
    var syntheticMemoryCleared = false
    InputiaMemoryBarrier.clear = { _ in syntheticMemoryCleared = true }
    try! InputiaVoiceServiceConnection.checkPolicy(InputiaVoicePolicyBarrier(barrier_id: String(repeating: "a", count: 64), version: nextVersion, clear_shared_personalization: true), minimumEpoch: 3, state: ClearedState()) { precondition(uiCleared && syntheticMemoryCleared) }
    InputiaMemoryBarrier.clear = nil
    global.didClear = nil
    let done = DispatchSemaphore(value: 0)
    DispatchQueue.global().async {
      do {
        for mode in ["valid", "new-generation", "bad-request", "bad-version", "expired", "no-capability"] {
          let applied = mode == "new-generation" ? nextVersion : version
          var fds: [Int32] = [0, 0]
          precondition(socketpair(AF_UNIX, SOCK_STREAM, 0, &fds) == 0)
          let connection = try InputiaVoiceServiceConnection.fixture(descriptor: fds[0], server:
            InputiaVoiceHello(protocol_major: 1, protocol_minor: 0, instance_id: "server", profile_id: "synthetic",
              policy_epoch: 3, capabilities: mode == "no-capability" ? ["voice_sessions_v1"] : ["voice_sessions_v1", "shared_terms_v1"]), version: applied)
          let peerFD = fds[1]
          let peerDone = DispatchSemaphore(value: 0)
          if mode != "no-capability" {
            DispatchQueue.global().async {
              do {
                let peer = try InputiaFramedConnection.fixture(descriptor: peerFD, timeout: 2)
                let request = try peer.read(Request.self)
                precondition(request.server_instance == "server" && request.client_instance == InputiaVoiceServiceConnection.processInstance)
                precondition(request.shared_terms.lease_id == "lease" && request.shared_terms.lease_epoch == 4)
                Thread.sleep(forTimeInterval: 0.05)
                try peer.write(Reply(request_id: mode == "bad-request" ? "wrong" : request.request_id,
                  lease_id: "lease", lease_epoch: 4,
                  version: mode == "bad-version" ? InputiaVoiceTermsVersion(policy_epoch: 3, learning_generation: 8) : applied,
                  max_age_ms: mode == "expired" ? 1 : 1000))
                peer.close()
              } catch { fatalError("synthetic shared terms peer failed") }
              peerDone.signal()
            }
          }
          let started = ProcessInfo.processInfo.systemUptime
          let lease = InputiaHostShortcutLease(lease_id: "lease", lease_epoch: 4, target: target(), issued_at_unix_ms: 0, expires_at_unix_ms: 1000)
          do {
            let result = try connection.fetchSharedTerms(lease: lease, leaseDeadline: started + 0.5)
            if ["valid", "new-generation"].contains(mode) {
              precondition(result?.terms.count == 1 && result!.expiresAt <= started + 0.5)
            } else { precondition(["expired", "no-capability"].contains(mode) && result == nil) }
          } catch { precondition(["bad-request", "bad-version"].contains(mode)) }
          connection.close()
          if mode == "no-capability" { Darwin.close(peerFD) }
          else { precondition(peerDone.wait(timeout: .now() + 3) == .success) }
        }
      } catch { fatalError("synthetic shared terms client failed") }
      done.signal()
    }
    precondition(done.wait(timeout: .now() + 8) == .success)
    print("shared_terms_memory_and_wire=pass cases=18 selection_intent_checks=10 chinese_mapping_checks=14 synthetic=true native_candidate_insertion_tested=false")
  }
}
