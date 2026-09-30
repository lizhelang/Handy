import Foundation
import CryptoKit

/// 旧学习域只交换短租约、查询绑定的值；这里不打开数据库，也不把响应当提交证明。
enum InputiaMemoryError: Error { case invalid, retired, unavailable, uncertain }
struct InputiaMemoryPolicy: Codable, Equatable {
  let server_instance: String
  let profile_id: String
  let policy_epoch: UInt64
}
struct InputiaMemoryTarget: Codable, Equatable {
  let target_id: String
  let host_instance: String
  let controller_id: String
  let activation_generation: UInt64
  let field_id: String?
  let selection_generation: UInt64
  let composition_generation: UInt64
  let source_app: String?
}
enum InputiaMemoryQuery: Codable, Equatable {
  case rank([String]), completion(String, Int), englishCompletion(String, Int), clipboard(Int), voiceHotwords(Int)
  var kind: String {
    switch self { case .rank: return "rank"; case .completion: return "completion"; case .englishCompletion: return "english_completion"; case .clipboard: return "clipboard"; case .voiceHotwords: return "voice_hotwords" }
  }
  enum CodingKeys: String, CodingKey { case kind, candidate_texts, prefix, limit }
  init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    switch try c.decode(String.self, forKey: .kind) {
    case "rank": self = .rank(try c.decode([String].self, forKey: .candidate_texts))
    case "completion": self = .completion(try c.decode(String.self, forKey: .prefix), try c.decode(Int.self, forKey: .limit))
    case "english_completion": self = .englishCompletion(try c.decode(String.self, forKey: .prefix), try c.decode(Int.self, forKey: .limit))
    case "clipboard": self = .clipboard(try c.decode(Int.self, forKey: .limit))
    case "voice_hotwords": self = .voiceHotwords(try c.decode(Int.self, forKey: .limit))
    default: throw InputiaMemoryError.invalid
    }
    try validate()
  }
  func encode(to encoder: Encoder) throws {
    try validate(); var c = encoder.container(keyedBy: CodingKeys.self); try c.encode(kind, forKey: .kind)
    switch self {
    case .rank(let values): try c.encode(values, forKey: .candidate_texts)
    case .completion(let prefix, let limit), .englishCompletion(let prefix, let limit): try c.encode(prefix, forKey: .prefix); try c.encode(limit, forKey: .limit)
    case .clipboard(let limit), .voiceHotwords(let limit): try c.encode(limit, forKey: .limit)
    }
  }
  func validate() throws {
    switch self {
    case .rank(let values):
      guard !values.isEmpty, values.count <= 64, values.reduce(0, { $0 + $1.utf8.count }) <= 65_536,
        values.allSatisfy({ !$0.isEmpty && Self.clean($0) }) else { throw InputiaMemoryError.invalid }
    case .completion(let prefix, let limit), .englishCompletion(let prefix, let limit):
      guard prefix.utf8.count <= 1024, Self.clean(prefix), (1...128).contains(limit) else { throw InputiaMemoryError.invalid }
    case .clipboard(let limit), .voiceHotwords(let limit): guard (1...128).contains(limit) else { throw InputiaMemoryError.invalid }
    }
  }
  static func clean(_ value: String) -> Bool { !value.unicodeScalars.contains { CharacterSet.controlCharacters.contains($0) } }
}
struct InputiaMemoryTerm: Codable, Equatable {
  let text: String
  let typed_count: UInt64
  let voice_count: UInt64
  let clipboard_count: UInt64
  let last_used_tick: UInt64
}
struct InputiaMemoryTicket: Codable, Equatable {
  let ticket: UInt64
  let query: InputiaMemoryQuery
  let composing: String
  let policy: InputiaMemoryPolicy
}
struct InputiaMemoryCommand: Codable {
  let kind: String
  var query_id: String? = nil
  var query_generation: UInt64? = nil
  var target: InputiaMemoryTarget? = nil
  var composing: String? = nil
  var query: InputiaMemoryQuery? = nil
  var operation_id: String? = nil
  var selection: String? = nil
  var limit: Int? = nil
  var request: InputiaMemoryFixedRequest? = nil
  var commit_id: String? = nil
  var plan_id: String? = nil
}

struct InputiaMemoryRange: Codable, Equatable { let location: UInt64; let length: UInt64 }
struct InputiaMemoryFixedPlan: Codable, Equatable { let candidate_id: String; let inserted_text: String }
struct InputiaMemoryFixedRequest: Codable, Equatable {
  let replacement: InputiaMemoryRange
  let replaced_text: String
  let retained_prefix: String
  let plans: [InputiaMemoryFixedPlan]
}
struct InputiaMemoryPreparedPlan: Codable { let candidate_id: String; let plan_id: String }
struct InputiaMemoryPermit: Codable {
  let commit_id: String
  let plans: [InputiaMemoryPreparedPlan]
  let max_age_ms: UInt64
  func validate(request: InputiaMemoryFixedRequest, started: TimeInterval, now: TimeInterval) throws {
    guard !commit_id.isEmpty, commit_id.utf8.count <= 128, InputiaMemoryQuery.clean(commit_id),
      (1...1500).contains(max_age_ms), now >= started, now - started < Double(max_age_ms) / 1000,
      plans.count == request.plans.count, !plans.isEmpty,
      Set(plans.map(\.candidate_id)) == Set(request.plans.map(\.candidate_id)),
      Set(plans.map(\.plan_id)).count == plans.count,
      plans.allSatisfy({ !$0.plan_id.isEmpty && $0.plan_id.utf8.count <= 128 && InputiaMemoryQuery.clean($0.plan_id) }) else { throw InputiaMemoryError.retired }
  }
  func operation(plan: String) -> String { "commit:\(commit_id):\(plan)" }
}
struct InputiaMemoryReceipt: Decodable {
  let operation_id: String
  let applied_at_epoch: UInt64
  let domain_uuid: String
  let generation: UInt64
  let state: String
  let replayed: Bool
}
struct InputiaMemoryRequest: Codable {
  let request_id: String
  let client_instance: String
  let server_instance: String
  let policy_epoch: UInt64
  let memory_domain: InputiaMemoryCommand

  /// 和 Rust legacy_memory_wire 完全相同的带长度二进制字段序列，不依赖 JSON key/转义顺序。
  func queryDigest() throws -> String {
    let command = memory_domain
    guard command.kind == "query", let id = command.query_id, let generation = command.query_generation,
      generation > 0, let target = command.target, let composing = command.composing, let query = command.query else { throw InputiaMemoryError.invalid }
    try query.validate()
    var data = Data("inputia-memory-query-v1\0".utf8)
    func number(_ value: UInt64) { var big = value.bigEndian; withUnsafeBytes(of: &big) { data.append(contentsOf: $0) } }
    func string(_ value: String) { number(UInt64(value.utf8.count)); data.append(contentsOf: value.utf8) }
    func optional(_ value: String?) { data.append(value == nil ? 0 : 1); if let value { string(value) } }
    string(request_id); string(client_instance); string(server_instance); number(policy_epoch)
    string(id); number(generation)
    string(target.target_id); string(target.host_instance); string(target.controller_id); number(target.activation_generation)
    optional(target.field_id); number(target.selection_generation); number(target.composition_generation); optional(target.source_app)
    string(composing); string(query.kind)
    switch query {
    case .rank(let values): number(UInt64(values.count)); values.forEach(string)
    case .completion(let prefix, let limit), .englishCompletion(let prefix, let limit): string(prefix); number(UInt64(limit))
    case .clipboard(let limit), .voiceHotwords(let limit): number(UInt64(limit))
    }
    return SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
  }
}
struct InputiaMemorySnapshot: Codable {
  let format_version: UInt32
  let request_id: String
  let server_instance: String
  let profile_id: String
  let policy_epoch: UInt64
  let domain_uuid: String
  let generation: UInt64
  let query_id: String
  let query_generation: UInt64
  let query_digest: String
  let query: InputiaMemoryQuery
  let terms: [InputiaMemoryTerm]
  let lease_id: String
  let max_age_ms: UInt64
  func validate(request: InputiaMemoryRequest, ticket: InputiaMemoryTicket, started: TimeInterval, now: TimeInterval) throws {
    let command = request.memory_domain
    guard format_version == 1, request_id == request.request_id, server_instance == request.server_instance,
      server_instance == ticket.policy.server_instance, profile_id == ticket.policy.profile_id,
      policy_epoch == request.policy_epoch, policy_epoch == ticket.policy.policy_epoch,
      query_id == command.query_id, query_generation == command.query_generation,
      query == command.query, query == ticket.query, ticket.composing == command.composing,
      query_digest == (try request.queryDigest()), generation > 0, !domain_uuid.isEmpty, lease_id == request.client_instance,
      domain_uuid.utf8.count <= 128, lease_id.utf8.count <= 128,
      (1...2000).contains(max_age_ms), now >= started, now - started < Double(max_age_ms) / 1000,
      terms.count <= 1024, terms.reduce(0, { $0 + $1.text.utf8.count }) <= 192 * 1024,
      Set(terms.map(\.text)).count == terms.count,
      terms.allSatisfy({ !$0.text.isEmpty && InputiaMemoryQuery.clean($0.text) }) else { throw InputiaMemoryError.retired }
  }
  func installation(ticket: InputiaMemoryTicket) -> InputiaMemoryInstall {
    .init(ticket: ticket.ticket, policy: ticket.policy, domain_uuid: domain_uuid, generation: generation,
      query: query, composing: ticket.composing, terms: terms, max_age_ms: max_age_ms)
  }
}
struct InputiaMemoryInstall: Encodable {
  let ticket: UInt64
  let policy: InputiaMemoryPolicy
  let domain_uuid: String
  let generation: UInt64
  let query: InputiaMemoryQuery
  let composing: String
  let terms: [InputiaMemoryTerm]
  let max_age_ms: UInt64
}
struct InputiaMemoryDomain: Decodable {
  let state: String
  let domain_uuid: String?
  let generation: UInt64
  let policy_epoch: UInt64
  let coverage: String
  let reason: String?
}
struct InputiaMemoryImportStatus: Codable {
  let operation_id: String
  let applied_at_epoch: UInt64
  let state: String
  let history_imported: UInt64
  let clipboard_imported: UInt64
  let skipped: UInt64
  let failure: String?
  var terminal: Bool { ["completed", "partial_failure", "revoked"].contains(state) }
}
struct InputiaMemoryOperationStatus: Decodable {
  let kind: String
  let operation_id: String
  let applied_at_epoch: UInt64
  let state: String
  let history_imported: UInt64?
  let clipboard_imported: UInt64?
  let skipped: UInt64?
  let failure: String?
  func imported() throws -> InputiaMemoryImportStatus {
    guard kind == "import", let history_imported, let clipboard_imported, let skipped else { throw InputiaMemoryError.invalid }
    return .init(operation_id: operation_id, applied_at_epoch: applied_at_epoch, state: state,
      history_imported: history_imported, clipboard_imported: clipboard_imported, skipped: skipped, failure: failure)
  }
}
struct InputiaMemoryResult: Decodable {
  let kind: String
  let domain: InputiaMemoryDomain?
  let snapshot: InputiaMemorySnapshot?
  // Import 与 Outcome 使用不同 tagged enum；保留原始 operation 供其类型精确解析。
  let operation: InputiaMemoryOperationPayload?
  let permit: InputiaMemoryPermit?
  let receipt: InputiaMemoryReceipt?
}
enum InputiaMemoryOperationPayload: Decodable {
  case imported(InputiaMemoryImportStatus), outcome(InputiaMemoryOperationStatus)
  init(from decoder: Decoder) throws {
    let key = try decoder.container(keyedBy: OperationKey.self)
    if key.contains(.kind) { self = .outcome(try InputiaMemoryOperationStatus(from: decoder)) }
    else { self = .imported(try InputiaMemoryImportStatus(from: decoder)) }
  }
  enum OperationKey: String, CodingKey { case kind }
  func imported() throws -> InputiaMemoryImportStatus { switch self { case .imported(let result): return result; case .outcome(let result): return try result.imported() } }
}

/// 主线程请求世代；每次真实输入、失活、屏障都会使尚未返回的查询退休。
final class InputiaMemoryRequestGeneration {
  private(set) var value: UInt64 = 1
  func cancelPending() { value = value == UInt64.max ? 1 : value + 1 }
  func accepts(_ expected: UInt64) -> Bool { value == expected }
}
enum InputiaMemoryInputTransition {
  static func retiresPending(keyDown: Bool, modeBoundary: Bool) -> Bool { keyDown || modeBoundary }
}
enum InputiaMemorySelectionAdmission {
  static func matches(expected: InputiaMemoryTarget, field: String, returned: InputiaMemoryTarget?,
                      server: String, returnedField: String?, ready: Bool, deadline: TimeInterval, now: TimeInterval) -> Bool {
    ready && returned == expected && returnedField.map { server + ":" + $0 == field } == true && now < deadline
  }
}

/// 主线程租约表。旧定时器只认识自己的identity，不得清掉同kind的新快照。
final class InputiaMemoryExpiryRegistry {
  private var entries: [String: (id: UUID, deadline: TimeInterval)] = [:]
  func install(kind: String, deadline: TimeInterval) -> UUID {
    let id = UUID(); entries[kind] = (id, deadline); return id
  }
  func retire(kind: String, identity: UUID, now: TimeInterval) -> Bool {
    guard let value = entries[kind], value.id == identity, now >= value.deadline else { return false }
    entries.removeAll(); return true
  }
  func clear() { entries.removeAll() }
}

/// 可撤销的排队payload。屏障同步释放尚未执行的正文闭包；在途工作必须再次核验token。
final class InputiaMemoryPendingWork {
  struct Token {
    fileprivate let owner: InputiaMemoryPendingWork
    fileprivate let epoch: UUID
    var valid: Bool { owner.isCurrent(epoch) }
  }
  private let lock = NSLock()
  private var epoch = UUID()
  private var work: [UUID: (run: (Token) -> Void, cancelled: () -> Void)] = [:]
  private func isCurrent(_ value: UUID) -> Bool { lock.lock(); defer { lock.unlock() }; return epoch == value }
  func enqueue(on queue: DispatchQueue, cancelled: @escaping () -> Void, run: @escaping (Token) -> Void) {
    let id = UUID()
    lock.lock(); let token = Token(owner: self, epoch: epoch); work[id] = (run, cancelled); lock.unlock()
    queue.async {
      self.lock.lock(); let item = self.work.removeValue(forKey: id); self.lock.unlock()
      guard let item else { return }
      if token.valid { item.run(token) } else { item.cancelled() }
    }
  }
  func cancelAll() {
    lock.lock(); epoch = UUID(); let callbacks = work.values.map(\.cancelled); work.removeAll(); lock.unlock()
    callbacks.forEach { $0() }
  }
}


/// 正式宿主在启动网络连接前安装同步清理器；未安装时屏障失败，不可回执成功。
enum InputiaMemoryBarrier {
  static var clear: ((InputiaMemoryPolicy) throws -> Void)?
  static var invalidate: (() -> Void)?
  static func apply(_ policy: InputiaMemoryPolicy) throws {
    guard Thread.isMainThread, let clear else { throw InputiaMemoryError.unavailable }
    try clear(policy)
  }
}
