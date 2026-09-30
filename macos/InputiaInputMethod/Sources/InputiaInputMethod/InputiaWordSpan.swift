import Foundation

struct InputiaWordSpanPermit: Codable {
  let span_id: String
  let max_age_ms: UInt64
  let max_units: Int
  let next_sequence: UInt64
}
struct InputiaWordSpanProgress: Codable {
  let span_id: String
  let sequence: UInt64
  let transcript_units: Int
  let replayed: Bool
}
enum InputiaWordSpanEdit: Codable, Equatable {
  case append(String), tailBackspace(UInt64)
  enum CodingKeys: String, CodingKey { case kind, text, units }
  init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    switch try c.decode(String.self, forKey: .kind) {
    case "append": self = .append(try c.decode(String.self, forKey: .text))
    case "tail_backspace": self = .tailBackspace(try c.decode(UInt64.self, forKey: .units))
    default: throw InputiaMemoryError.invalid
    }
  }
  func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: CodingKeys.self)
    switch self {
    case .append(let text): try c.encode("append", forKey: .kind); try c.encode(text, forKey: .text)
    case .tailBackspace(let units): try c.encode("tail_backspace", forKey: .kind); try c.encode(units, forKey: .units)
    }
  }
}
struct InputiaWordSpanContext: Equatable {
  let target: InputiaMemoryTarget
  let policy: InputiaMemoryPolicy
  let field: String
  let owner: String
  let activation: UInt64
  let caret: UInt64
}
struct InputiaWordSpanRecord: Equatable {
  let sequence: UInt64
  let edit: InputiaWordSpanEdit
  let expectedUnits: Int
}

/// 只记录获得许可以后的可观察编辑；不读取、导入或补齐此前键入的前缀。
struct InputiaWordSpanState {
  let permit: InputiaWordSpanPermit
  let context: InputiaWordSpanContext
  private(set) var deadline: TimeInterval
  private(set) var sequence: UInt64 = 0
  private(set) var acknowledged: UInt64 = 0
  private(set) var sealing = false
  private(set) var records: [InputiaWordSpanRecord] = []
  private var text: [UInt8] = []
  var units: Int { text.count }
  var caret: UInt64 { context.caret + UInt64(text.count) }
  var closedBoundary: Bool { text.last.map { !Self.wordUnit($0) } ?? false }
  static func wordUnit(_ byte: UInt8) -> Bool {
    (48...57).contains(byte) || (65...90).contains(byte) || (97...122).contains(byte) || byte == 95 || byte == 45
  }
  init(permit: InputiaWordSpanPermit, context: InputiaWordSpanContext, started: TimeInterval, now: TimeInterval) throws {
    guard !permit.span_id.isEmpty, permit.span_id.utf8.count <= 128, InputiaMemoryQuery.clean(permit.span_id),
      (1...1500).contains(permit.max_age_ms), (1...8192).contains(permit.max_units), permit.next_sequence == 1,
      !context.field.isEmpty, !context.owner.isEmpty, context.policy.policy_epoch > 0,
      context.target.field_id != nil, context.target.host_instance != "",
      context.caret <= UInt64(Int.max - 8192), now >= started,
      now < started + Double(permit.max_age_ms) / 1000 else { throw InputiaMemoryError.retired }
    self.permit = permit; self.context = context; deadline = started + Double(permit.max_age_ms) / 1000
  }
  mutating func record(_ edit: InputiaWordSpanEdit, before: UInt64, after: UInt64, now: TimeInterval) throws {
    guard !sealing, now < deadline, before == caret, records.count < 32, sequence < 1024 else { throw InputiaMemoryError.retired }
    var next = text
    switch edit {
    case .append(let value):
      let bytes = Array(value.utf8)
      guard !bytes.isEmpty, bytes.count <= 64, bytes.allSatisfy({ (32...126).contains($0) }) else { throw InputiaMemoryError.invalid }
      next.append(contentsOf: bytes)
    case .tailBackspace(let count):
      guard count > 0, count <= UInt64(next.count) else { throw InputiaMemoryError.invalid }
      next.removeLast(Int(count))
    }
    guard next.count <= permit.max_units, after == context.caret + UInt64(next.count) else { throw InputiaMemoryError.retired }
    text = next; sequence += 1
    records.append(.init(sequence: sequence, edit: edit, expectedUnits: next.count))
  }
  mutating func acknowledge(_ progress: InputiaWordSpanProgress) throws {
    guard let first = records.first, progress.span_id == permit.span_id,
      progress.sequence == first.sequence, progress.transcript_units == first.expectedUnits else { throw InputiaMemoryError.retired }
    acknowledged = progress.sequence; records.removeFirst()
  }
  func checkpointOperation(finish: Bool) throws -> String {
    guard records.isEmpty, sequence > 0, acknowledged == sequence, !finish || closedBoundary else { throw InputiaMemoryError.retired }
    return "word-span:\(permit.span_id):\(sequence)\(finish ? ":seal" : "")"
  }
  mutating func markSealing() { sealing = true }
  mutating func renewed(started: TimeInterval, now: TimeInterval) throws {
    let next = started + Double(permit.max_age_ms) / 1000
    guard now >= started, now < next else { throw InputiaMemoryError.retired }; deadline = next
  }
}

/// 由已认证 transport 转换的回复。自检可注入合成回复，但不能充当原生字段证明。
struct InputiaWordSpanResponse {
  let policy: InputiaMemoryPolicy
  let kind: String
  var permit: InputiaWordSpanPermit? = nil
  var progress: InputiaWordSpanProgress? = nil
  var receipt: InputiaMemoryReceipt? = nil
}

/// 主线程协调器：record逐个确认，checkpoint只在此前record全部确认后发送。
final class InputiaWordSpan {
  typealias Send = (InputiaMemoryCommand, InputiaMemoryPolicy, @escaping (Result<InputiaWordSpanResponse, Error>) -> Void) -> Void
  private let send: Send
  private let current: (InputiaWordSpanContext, UInt64) -> Bool
  private let ended: (InputiaWordSpanContext) -> Void
  private let clock: () -> TimeInterval
  private var generation = UUID()
  private var preparing: InputiaWordSpanContext?
  private(set) var state: InputiaWordSpanState?
  private var sending = false
  private var checkpointPending = false
  private var checkpointInFlight = false
  private var finishPending = false
  private var idle: DispatchWorkItem?
  private var expiry: DispatchWorkItem?
  private(set) var coverageReason = "no_permit"
  var hasPermit: Bool { state != nil && !checkpointInFlight && state?.sealing == false }
  var isBusy: Bool { state != nil || preparing != nil }
  init(send: @escaping Send, current: @escaping (InputiaWordSpanContext, UInt64) -> Bool,
       ended: @escaping (InputiaWordSpanContext) -> Void, clock: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) {
    self.send = send; self.current = current; self.ended = ended; self.clock = clock
  }
  func prepare(_ context: InputiaWordSpanContext) {
    guard !isBusy, current(context, context.caret) else { return }
    let ticket = UUID(); generation = ticket; preparing = context
    let started = clock()
    send(.init(kind: "prepare_word_span", target: context.target), context.policy) { [weak self] result in
      guard let self else { return }
      guard case .success(let reply) = result, reply.policy == context.policy,
        reply.kind == "prepared_word_span", let permit = reply.permit else {
        if self.generation == ticket { self.preparing = nil; self.coverageReason = "prepare_unavailable"; self.ended(context) }; return
      }
      guard self.generation == ticket, self.preparing != nil, self.current(context, context.caret) else {
        self.retireMetadata(permit.span_id, context: context); return
      }
      self.preparing = nil
      do {
        self.state = try .init(permit: permit, context: context, started: started, now: self.clock())
        self.coverageReason = "observing"; self.scheduleExpiry()
      } catch { self.retireMetadata(permit.span_id, context: context); self.coverageReason = "late_permit" }
    }
  }
  /// 必须在本次真实编辑发生后调用；早于许可的字符会使准备退休，绝不补进新span。
  func observed(_ edit: InputiaWordSpanEdit, before: UInt64, after: UInt64) {
    guard var value = state, !checkpointInFlight, current(value.context, after) else { retire(reason: "uncovered_or_unverified_edit"); return }
    do {
      try value.record(edit, before: before, after: after, now: clock()); state = value
      idle?.cancel()
      if value.closedBoundary { finishPending = true; value.markSealing(); state = value; checkpointPending = true }
      else {
        let identity = generation
        let timer = DispatchWorkItem { [weak self] in
          guard let self, self.generation == identity else { return }
          self.checkpointPending = true; self.pump()
        }
        idle = timer; DispatchQueue.main.asyncAfter(deadline: .now() + 0.15, execute: timer)
      }
      pump()
    } catch { retire(reason: "edit_boundary_or_budget") }
  }
  func retire(reason: String) {
    generation = UUID(); idle?.cancel(); expiry?.cancel(); idle = nil; expiry = nil
    let old = state; let pending = preparing
    state = nil; preparing = nil; sending = false; checkpointPending = false; checkpointInFlight = false; finishPending = false
    coverageReason = reason
    if let old { retireMetadata(old.permit.span_id, context: old.context) }
    else if let pending { ended(pending) }
  }
  private func retireMetadata(_ span: String, context: InputiaWordSpanContext) {
    // 这里只发ID，允许同server/client的旧epoch撤销；无正文、无重新学习。
    send(.init(kind: "retire_word_span", span_id: span), context.policy) { [weak self] _ in self?.ended(context) }
  }
  private func scheduleExpiry() {
    expiry?.cancel()
    guard let value = state else { return }
    let identity = generation
    let timer = DispatchWorkItem { [weak self] in
      guard let self, self.generation == identity, let value = self.state else { return }
      guard self.clock() >= value.deadline else { self.scheduleExpiry(); return }
      self.retire(reason: "expired")
    }
    expiry = timer; DispatchQueue.main.asyncAfter(deadline: .now() + max(0, value.deadline - clock()), execute: timer)
  }
  private func pump() {
    guard !sending, let value = state else { return }
    guard clock() < value.deadline, current(value.context, value.caret) else { retire(reason: "scope_or_lease_changed"); return }
    let identity = generation, context = value.context
    if let record = value.records.first {
      sending = true
      send(.init(kind: "record_word_span", target: value.context.target, span_id: value.permit.span_id,
        sequence: record.sequence, edit: record.edit), value.context.policy) { [weak self] result in
        guard let self, self.generation == identity else { return }
        self.sending = false
        guard case .success(let reply) = result, reply.policy == context.policy, reply.kind == "word_span_progress",
          let progress = reply.progress else { self.retire(reason: "record_unconfirmed"); return }
        do { try self.state?.acknowledge(progress); self.pump() }
        catch { self.retire(reason: "record_mismatch") }
      }
      return
    }
    guard checkpointPending else { return }
    do {
      let finish = finishPending, operation = try value.checkpointOperation(finish: finish), started = clock()
      sending = true; checkpointPending = false; checkpointInFlight = true
      send(.init(kind: "checkpoint_word_span", target: value.context.target, operation_id: operation,
        span_id: value.permit.span_id, through_sequence: value.sequence, finish: finish), value.context.policy) { [weak self] result in
        guard let self, self.generation == identity else { return }
        self.sending = false; self.checkpointInFlight = false
        guard case .success(let reply) = result, reply.policy == context.policy, reply.kind == "learn", let receipt = reply.receipt,
          receipt.operation_id == operation, receipt.applied_at_epoch == context.policy.policy_epoch,
          ["applied", "already_contributed"].contains(receipt.state), receipt.generation > 0, !receipt.domain_uuid.isEmpty else {
          self.retire(reason: "readback_unconfirmed"); return
        }
        if finish {
          self.state = nil; self.expiry?.cancel(); self.idle?.cancel(); self.finishPending = false
          self.coverageReason = "sealed"; self.generation = UUID(); self.ended(context)
        } else {
          do { try self.state?.renewed(started: started, now: self.clock()); self.scheduleExpiry(); self.pump() }
          catch { self.retire(reason: "renewal_late") }
        }
      }
    } catch { retire(reason: "checkpoint_not_ready") }
  }
}
