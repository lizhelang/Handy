import Foundation

@main
struct InputiaWordSpanSelfCheck {
  static func main() throws {
    var checks = 0
    func check(_ value: Bool) { precondition(value, "check \(checks + 1)"); checks += 1 }
    let policy = InputiaMemoryPolicy(server_instance: "server", profile_id: "profile", policy_epoch: 3)
    let context = InputiaWordSpanContext(target: .init(target_id: "target", host_instance: "host", controller_id: "controller",
      activation_generation: 2, field_id: "target", selection_generation: 1, composition_generation: 1, source_app: "app"),
      policy: policy, field: "server:field", owner: "owner", activation: 2, caret: 10)
    let permit = InputiaWordSpanPermit(span_id: "span", max_age_ms: 1500, max_units: 8192, next_sequence: 1)
    func state() throws -> InputiaWordSpanState { try .init(permit: permit, context: context, started: 0, now: 0.1) }
    check((try? InputiaWordSpanState(permit: permit, context: context, started: 0, now: 1.5)) == nil)
    var value = try state()
    try value.record(.append("he"), before: 10, after: 12, now: 0.2)
    check(value.records == [.init(sequence: 1, edit: .append("he"), expectedUnits: 2)])
    check((try? value.checkpointOperation(finish: false)) == nil)
    check((try? value.acknowledge(.init(span_id: "other", sequence: 1, transcript_units: 2, replayed: false))) == nil)
    try value.acknowledge(.init(span_id: "span", sequence: 1, transcript_units: 2, replayed: false))
    check(try value.checkpointOperation(finish: false) == "word-span:span:1")
    check((try? value.checkpointOperation(finish: true)) == nil)
    try value.record(.tailBackspace(1), before: 12, after: 11, now: 0.3)
    try value.acknowledge(.init(span_id: "span", sequence: 2, transcript_units: 1, replayed: false))
    try value.record(.append("i "), before: 11, after: 13, now: 0.4)
    try value.acknowledge(.init(span_id: "span", sequence: 3, transcript_units: 3, replayed: false))
    check(try value.checkpointOperation(finish: true) == "word-span:span:3:seal")
    value.markSealing()
    check((try? value.record(.append("x"), before: 13, after: 14, now: 0.5)) == nil)
    var wrongCaret = try state()
    check((try? wrongCaret.record(.append("x"), before: 10, after: 15, now: 0.2)) == nil)
    check(wrongCaret.units == 0 && wrongCaret.sequence == 0)
    check((try? wrongCaret.record(.tailBackspace(1), before: 10, after: 9, now: 0.2)) == nil)
    check((try? wrongCaret.record(.append("中"), before: 10, after: 11, now: 0.2)) == nil)
    check((try? wrongCaret.record(.append("\n"), before: 10, after: 11, now: 0.2)) == nil)
    var bounded = try state()
    for i in 0..<32 { try bounded.record(.append("a"), before: UInt64(10+i), after: UInt64(11+i), now: 0.2) }
    check((try? bounded.record(.append("a"), before: 42, after: 43, now: 0.2)) == nil)
    let tiny = InputiaWordSpanPermit(span_id: "tiny", max_age_ms: 1500, max_units: 1, next_sequence: 1)
    var limited = try InputiaWordSpanState(permit: tiny, context: context, started: 0, now: 0.1)
    check((try? limited.record(.append("aa"), before: 10, after: 12, now: 0.2)) == nil)
    let raw = #"{"kind":"prepared_word_span","permit":{"span_id":"s","max_age_ms":1500,"max_units":8192,"next_sequence":1}}"#
    let decoded = try JSONDecoder().decode(InputiaMemoryResult.self, from: Data(raw.utf8))
    check(decoded.wordSpanPermit?.span_id == "s" && decoded.permit == nil)
    check((try? JSONDecoder().decode(InputiaMemoryResult.self, from: Data(raw.replacingOccurrences(of: "prepared_word_span", with: "prepared_commit").utf8))) == nil)
    check((try? JSONDecoder().decode(InputiaMemoryResult.self, from: Data(#"{"kind":"unknown"}"#.utf8))) == nil)

    final class Fake {
      var requests: [(InputiaMemoryCommand, InputiaMemoryPolicy, (Result<InputiaWordSpanResponse, Error>) -> Void)] = []
      var ended = 0
      var caret: UInt64 = 10
      var scope = true
      var now = 0.0
      func send(_ command: InputiaMemoryCommand, _ policy: InputiaMemoryPolicy, _ done: @escaping (Result<InputiaWordSpanResponse, Error>) -> Void) { requests.append((command, policy, done)) }
      func reply(_ index: Int, _ reply: InputiaWordSpanResponse) { requests[index].2(.success(reply)) }
    }
    func coordinator(_ f: Fake) -> InputiaWordSpan {
      .init(send: f.send, current: { _, caret in f.scope && f.caret == caret }, ended: { _ in f.ended += 1 }, clock: { f.now })
    }
    let early = Fake()
    let e = coordinator(early)
    e.prepare(context); early.caret = 11; e.observed(.append("a"), before: 10, after: 11)
    early.reply(0, .init(policy: policy, kind: "prepared_word_span", permit: permit))
    check(!e.hasPermit && early.requests.count == 2 && early.requests[1].0.kind == "retire_word_span")
    check(early.requests[1].0.target == nil && early.requests[1].0.edit == nil)

    let f = Fake(), c = coordinator(f)
    c.prepare(context); f.reply(0, .init(policy: policy, kind: "prepared_word_span", permit: permit))
    f.caret = 11; c.observed(.append("h"), before: 10, after: 11)
    f.caret = 12; c.observed(.append("i"), before: 11, after: 12)
    f.caret = 13; c.observed(.append(" "), before: 12, after: 13)
    check(f.requests.count == 2 && f.requests[1].0.sequence == 1)
    for index in 1...3 {
      f.reply(index, .init(policy: policy, kind: "word_span_progress", progress: .init(span_id: "span", sequence: UInt64(index), transcript_units: index, replayed: false)))
    }
    check(f.requests.count == 5 && f.requests[4].0.kind == "checkpoint_word_span")
    check(f.requests[4].0.operation_id == "word-span:span:3:seal" && f.requests[4].0.finish == true)
    f.reply(4, .init(policy: policy, kind: "learn", receipt: .init(operation_id: "word-span:span:3:seal", applied_at_epoch: 3,
      domain_uuid: "domain", generation: 1, state: "applied", replayed: false)))
    check(c.state == nil && c.coverageReason == "sealed" && f.ended == 1)
    f.caret = 14; c.observed(.append("x"), before: 13, after: 14)
    check(f.requests.count == 5) // 封存后的字符不追加，也不事后生成许可。

    let bad = Fake(), badSpan = coordinator(bad)
    badSpan.prepare(context); bad.reply(0, .init(policy: policy, kind: "prepared_word_span", permit: permit))
    bad.caret = 11; badSpan.observed(.append("x"), before: 10, after: 11)
    bad.reply(1, .init(policy: policy, kind: "word_span_progress", progress: .init(span_id: "span", sequence: 2, transcript_units: 1, replayed: false)))
    check(badSpan.state == nil && bad.requests.last?.0.kind == "retire_word_span")
    let changed = Fake(), changedSpan = coordinator(changed)
    changedSpan.prepare(context); changed.reply(0, .init(policy: policy, kind: "prepared_word_span", permit: permit))
    changed.scope = false; changed.caret = 11; changedSpan.observed(.append("x"), before: 10, after: 11)
    check(changedSpan.state == nil && changed.requests.last?.1 == policy)
    let revoked = Fake(), revokedSpan = coordinator(revoked)
    revokedSpan.prepare(context); revoked.reply(0, .init(policy: policy, kind: "prepared_word_span", permit: permit))
    revokedSpan.retire(reason: "barrier")
    check(revokedSpan.state == nil && revoked.requests.last?.0.span_id == "span" && revoked.requests.last?.1.policy_epoch == 3)

    // 使用真实主线程时钟检查空闲截止计时器，不需要输入事件驱动清理。
    let timeout = Fake()
    let timed = InputiaWordSpan(send: timeout.send, current: { _, _ in true }, ended: { _ in timeout.ended += 1 })
    timed.prepare(context)
    timeout.reply(0, .init(policy: policy, kind: "prepared_word_span", permit: .init(span_id: "short", max_age_ms: 20, max_units: 8192, next_sequence: 1)))
    RunLoop.main.run(until: Date().addingTimeInterval(0.04))
    check(timed.state == nil && timed.coverageReason == "expired" && timeout.requests.last?.0.kind == "retire_word_span")

    // active checkpoint 只有真实receipt才续租；checkpoint在途的新编辑停止学习，不阻塞编辑调用方。
    let active = Fake(), activeSpan = coordinator(active)
    activeSpan.prepare(context); active.reply(0, .init(policy: policy, kind: "prepared_word_span", permit: permit))
    active.caret = 12; activeSpan.observed(.append("hi"), before: 10, after: 12)
    active.reply(1, .init(policy: policy, kind: "word_span_progress", progress: .init(span_id: "span", sequence: 1, transcript_units: 2, replayed: false)))
    active.now = 0.7
    RunLoop.main.run(until: Date().addingTimeInterval(0.17))
    check(active.requests.last?.0.operation_id == "word-span:span:1" && active.requests.last?.0.finish == false)
    active.now = 0.8
    active.reply(2, .init(policy: policy, kind: "learn", receipt: .init(operation_id: "word-span:span:1", applied_at_epoch: 3,
      domain_uuid: "domain", generation: 1, state: "applied", replayed: false)))
    check(activeSpan.hasPermit && activeSpan.state?.deadline == 2.2)
    active.caret = 13; activeSpan.observed(.append(" "), before: 12, after: 13)
    active.reply(3, .init(policy: policy, kind: "word_span_progress", progress: .init(span_id: "span", sequence: 2, transcript_units: 3, replayed: false)))
    check(active.requests.last?.0.operation_id == "word-span:span:2:seal")
    active.reply(4, .init(policy: policy, kind: "learn", receipt: .init(operation_id: "other", applied_at_epoch: 3,
      domain_uuid: "domain", generation: 1, state: "applied", replayed: false)))
    check(activeSpan.state == nil && activeSpan.coverageReason == "readback_unconfirmed")
    let inFlight = Fake(), inFlightSpan = coordinator(inFlight)
    inFlightSpan.prepare(context); inFlight.reply(0, .init(policy: policy, kind: "prepared_word_span", permit: permit))
    inFlight.caret = 12; inFlightSpan.observed(.append("hi"), before: 10, after: 12)
    inFlight.reply(1, .init(policy: policy, kind: "word_span_progress", progress: .init(span_id: "span", sequence: 1, transcript_units: 2, replayed: false)))
    RunLoop.main.run(until: Date().addingTimeInterval(0.17))
    check(!inFlightSpan.hasPermit && inFlight.requests[2].0.kind == "checkpoint_word_span")
    inFlight.caret = 13; inFlightSpan.observed(.append("x"), before: 12, after: 13)
    check(inFlightSpan.state == nil && inFlight.caret == 13 && inFlight.requests.last?.0.kind == "retire_word_span")
    inFlight.reply(2, .init(policy: policy, kind: "learn", receipt: .init(operation_id: "word-span:span:1", applied_at_epoch: 3,
      domain_uuid: "domain", generation: 1, state: "applied", replayed: false)))
    check(inFlightSpan.state == nil) // 已退休的迟到确认不能重新装回正文/许可。
    let stale = Fake()
    let staleSpan = InputiaWordSpan(send: stale.send, current: { _, _ in true }, ended: { _ in })
    staleSpan.prepare(context)
    stale.reply(0, .init(policy: policy, kind: "prepared_word_span", permit: .init(span_id: "old", max_age_ms: 20, max_units: 8192, next_sequence: 1)))
    staleSpan.retire(reason: "new_scope"); staleSpan.prepare(context)
    stale.reply(2, .init(policy: policy, kind: "prepared_word_span", permit: .init(span_id: "new", max_age_ms: 200, max_units: 8192, next_sequence: 1)))
    RunLoop.main.run(until: Date().addingTimeInterval(0.04))
    check(staleSpan.state?.permit.span_id == "new")
    staleSpan.retire(reason: "fixture_end")
    let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(f.requests[1].0)) as! [String: Any]
    check(encoded["kind"] as? String == "record_word_span" && encoded["span_id"] as? String == "span")
    check((encoded["edit"] as? [String: String]) == ["kind": "append", "text": "h"])
    print("wordSpanSelfCheck=PASS checks=\(checks)")
  }
}
