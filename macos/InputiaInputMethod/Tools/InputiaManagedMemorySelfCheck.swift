import Foundation

@main
struct InputiaManagedMemorySelfCheck {
  static func main() throws {
    struct Vector: Decodable { let request: InputiaMemoryRequest; let sha256: String }
    let vectors = try JSONDecoder().decode([Vector].self, from: Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1])))
    var checks = 0
    func check(_ result: Bool) { precondition(result); checks += 1 }
    for vector in vectors { check(try vector.request.queryDigest() == vector.sha256) }
    let generation = InputiaMemoryRequestGeneration(); let first = generation.value
    check(generation.accepts(first)); generation.cancelPending(); check(!generation.accepts(first))
    let request = vectors[0].request
    let command = request.memory_domain
    let ticket = InputiaMemoryTicket(ticket: 1, query: command.query!, composing: command.composing!, policy: .init(server_instance: request.server_instance, profile_id: "profile-1", policy_epoch: request.policy_epoch))
    let snapshot = InputiaMemorySnapshot(format_version: 1, request_id: request.request_id, server_instance: request.server_instance,
      profile_id: "profile-1", policy_epoch: request.policy_epoch, domain_uuid: "domain-1", generation: 1,
      query_id: command.query_id!, query_generation: command.query_generation!, query_digest: try request.queryDigest(),
      query: command.query!, terms: [.init(text: "你好", typed_count: 1, voice_count: 0, clipboard_count: 0, last_used_tick: 1)], lease_id: request.client_instance, max_age_ms: 100)
    try snapshot.validate(request: request, ticket: ticket, started: 10, now: 10.05); checks += 1
    check((try? snapshot.validate(request: request, ticket: ticket, started: 10, now: 10.2)) == nil)
    let wrong = InputiaMemoryTicket(ticket: 1, query: command.query!, composing: "changed", policy: ticket.policy)
    check((try? snapshot.validate(request: request, ticket: wrong, started: 10, now: 10.05)) == nil)
    let wrongProfile = InputiaMemoryTicket(ticket: 1, query: command.query!, composing: command.composing!, policy: .init(server_instance: request.server_instance, profile_id: "other", policy_epoch: 2))
    check((try? snapshot.validate(request: request, ticket: wrongProfile, started: 10, now: 10.05)) == nil)
    for raw in [#"{"kind":"rank","candidate_texts":[]}"#, #"{"kind":"clipboard","limit":0}"#, #"{"kind":"voice_hotwords","limit":129}"#] {
      check((try? JSONDecoder().decode(InputiaMemoryQuery.self, from: Data(raw.utf8))) == nil)
    }
    let fixed = InputiaMemoryFixedRequest(replacement: .init(location: 3, length: 0), replaced_text: "",
      retained_prefix: "inp", plans: [.init(candidate_id: "c1", inserted_text: "utia")])
    let permit = InputiaMemoryPermit(commit_id: "commit", plans: [.init(candidate_id: "c1", plan_id: "plan")], max_age_ms: 1500)
    check((try? permit.validate(request: fixed, started: 10, now: 11)) != nil)
    check((try? permit.validate(request: fixed, started: 10, now: 11.5)) == nil)
    check(permit.operation(plan: "plan") == "commit:commit:plan")
    let swapped = InputiaMemoryPermit(commit_id: "commit", plans: [.init(candidate_id: "other", plan_id: "plan")], max_age_ms: 1500)
    check((try? swapped.validate(request: fixed, started: 10, now: 11)) == nil)
    let repeated = InputiaMemoryPermit(commit_id: "commit", plans: [.init(candidate_id: "c1", plan_id: "p"), .init(candidate_id: "c1", plan_id: "p")], max_age_ms: 1500)
    check((try? repeated.validate(request: fixed, started: 10, now: 11)) == nil)
    let idle = InputiaMemoryExpiryRegistry()
    let oldLease = idle.install(kind: "clipboard", deadline: 12)
    check(!idle.retire(kind: "clipboard", identity: oldLease, now: 11))
    let freshLease = idle.install(kind: "clipboard", deadline: 14)
    check(!idle.retire(kind: "clipboard", identity: oldLease, now: 12))
    check(idle.retire(kind: "clipboard", identity: freshLease, now: 14))
    check(!idle.retire(kind: "clipboard", identity: freshLease, now: 15))
    let visibleGeneration = generation.value
    if InputiaMemoryInputTransition.retiresPending(keyDown: false, modeBoundary: false) { generation.cancelPending() }
    check(generation.accepts(visibleGeneration)) // 松键不改变查询/许可代数。
    check((try? permit.validate(request: fixed, started: 10, now: 11)) != nil)
    if InputiaMemoryInputTransition.retiresPending(keyDown: true, modeBoundary: false) { generation.cancelPending() }
    check(!generation.accepts(visibleGeneration))
    let target = command.target!
    check(InputiaMemorySelectionAdmission.matches(expected: target, field: "server:fieldA", returned: target,
      server: "server", returnedField: "fieldA", ready: true, deadline: 12, now: 11))
    check(!InputiaMemorySelectionAdmission.matches(expected: target, field: "server:fieldA", returned: target,
      server: "server", returnedField: "fieldB", ready: true, deadline: 12, now: 11))
    check(!InputiaMemorySelectionAdmission.matches(expected: target, field: "server:fieldA", returned: target,
      server: "other", returnedField: "fieldA", ready: true, deadline: 12, now: 11))
    let pendingWork = InputiaMemoryPendingWork(), queue = DispatchQueue(label: "memory-fixture-queue")
    queue.suspend()
    var sent = 0, cancelled = 0
    pendingWork.enqueue(on: queue, cancelled: { cancelled += 1 }) { _ in sent += 1 }
    pendingWork.cancelAll(); queue.resume(); queue.sync {}
    check(sent == 0 && cancelled == 1)
    var inFlight: InputiaMemoryPendingWork.Token?
    pendingWork.enqueue(on: queue, cancelled: {}) { token in inFlight = token }
    queue.sync {}; check(inFlight?.valid == true)
    pendingWork.cancelAll(); check(inFlight?.valid == false)
    // 复用宿主实际deferral：已消费的Space准入尚未回复时，下一普通键必须保序。
    var deferred = InputiaPersonalInputDeferral<String>(), inserted: [String] = []
    let admission = deferred.begin(now: 20)!
    if case .queued = deferred.offer("x", now: 20.1, scopeMatches: true, boundary: false) { checks += 1 }
    else { fatalError("next input bypassed pending candidate admission") }
    check(deferred.token == admission && inserted.isEmpty)
    let exactField = InputiaMemorySelectionAdmission.matches(expected: target, field: "server:fieldA", returned: target,
      server: "server", returnedField: "fieldA", ready: true, deadline: 20.5, now: 20.2)
    if exactField, let queued = deferred.finish(admission) { inserted.append("候选"); inserted.append(contentsOf: queued) }
    check(inserted == ["候选", "x"] && deferred.finish(admission) == nil)
    print("managedMemorySelfCheck=PASS checks=\(checks)")
  }
}
