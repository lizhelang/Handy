#if INPUTIA_PAIRED_BUILD
import Foundation
import SQLite3

/// Wire DTO字段与Rust JSON合同一致，不用于系统输入框或全文日志。
struct InputiaVoiceHello: Codable, Equatable {
  let protocol_major: UInt16
  let protocol_minor: UInt16
  let instance_id: String
  let profile_id: String
  let policy_epoch: UInt64
  let capabilities: [String]
  var pair_binding: InputiaPairBinding? = nil
}

struct InputiaMenuModel: Decodable {
  let id: String
  let name: String
  let available: Bool
}
struct InputiaMenuReply: Decodable {
  let status: String
  let request_id: String
  let selected_model: String?
  let models: [InputiaMenuModel]?
  let busy: Bool?
  let clipboard_hotkey: String?
  let clipboard_hotkey_enabled: Bool?
  let code: String?
}
private struct InputiaMenuCommand: Encodable {
  let kind: String
  let model_id: String?
}
private struct InputiaMenuRequest: Encodable {
  let request_id: String
  let client_instance: String
  let server_instance: String
  let policy_epoch: UInt64
  let menu: InputiaMenuCommand
}
private struct InputiaVoiceHelloReply: Decodable {
  let status: String
  let server: InputiaVoiceHello?
  let negotiated_minor: UInt16?
  let require_policy_refresh: Bool?
}
struct InputiaVoiceTermsVersion: Codable, Equatable {
  let policy_epoch: UInt64
  let learning_generation: UInt64
}

struct InputiaSharedTermsSnapshot {
  let identity: String
  let target: InputiaVoiceTarget
  let version: InputiaVoiceTermsVersion
  let terms: [String]
  var explicitTerms: [String] = []
  let expiresAt: TimeInterval
  func englishCandidates(prefix: String) -> [String] {
    guard prefix.count >= 2 else { return [] }
    let explicit = InputiaHotwordPrefix.candidates(explicitTerms, code: prefix).filter {
      $0.count > prefix.count && $0.lowercased().hasPrefix(prefix.lowercased())
    }
    return explicit + terms.filter { term in
      !explicitTerms.contains(term) &&
      term.count > prefix.count && term.count <= 32 && term.lowercased().hasPrefix(prefix.lowercased())
        && term.unicodeScalars.allSatisfy { scalar in
          (48...57).contains(scalar.value) || (65...90).contains(scalar.value)
            || (97...122).contains(scalar.value) || scalar.value == 95 || scalar.value == 45
        }
        && term.unicodeScalars.contains { (65...90).contains($0.value) || (97...122).contains($0.value) }
    }
  }
}

/// 显式热词前缀匹配；只改变展示，不构造 Rime 候选身份。
enum InputiaHotwordPrefix {
  static func candidates(_ terms: [String], code: String, naturalDoublePinyin: Bool = true) -> [String] {
    let normalized = code.lowercased().filter { $0 != " " && $0 != "'" }
    guard !normalized.isEmpty else { return [] }
    return terms.filter { term in
      let first = Array(term.prefix(3))
      if first.count == 3, first.allSatisfy({ $0.isASCII && $0.isLetter }), normalized.count == 3 {
        return String(first).lowercased() == normalized
      }
      let chinese = Array(term.prefix(2))
      guard chinese.count == 2, chinese.allSatisfy({ character in
        character.unicodeScalars.allSatisfy { (0x3400...0x9fff).contains($0.value) }
      }), let latin = String(chinese).applyingTransform(.toLatin, reverse: false) else { return false }
      let umlauts = latin.lowercased().replacingOccurrences(of: "[üǖǘǚǜ]", with: "v", options: .regularExpression)
      guard let unaccented = umlauts.applyingTransform(.stripDiacritics, reverse: false) else { return false }
      let syllables = unaccented.split(separator: " ").map(String.init)
      guard syllables.count == 2 else { return false }
      return normalized == syllables.joined() || (naturalDoublePinyin && normalized == syllables.map(naturalCode).joined())
    }
  }
  static func naturalCode(_ syllable: String) -> String {
    var value = syllable.replacingOccurrences(of: "ü", with: "v")
    // 与自然码 schema 的顺序变换保持一致，保留零声母音节的规则。
    let rules = [("^([aoe])([ioun])$", "$1$1$2"), ("^([aoe])(ng)?$", "$1$1$2"),
      ("iu$", "Q"), ("[iu]a$", "W"), ("[uv]an$", "R"), ("[uv]e$", "T"),
      ("ing$|uai$", "Y"), ("^sh", "U"), ("^ch", "I"), ("^zh", "V"),
      ("uo$", "O"), ("[uv]n$", "P"), ("i?ong$", "S"), ("[iu]ang$", "D"),
      ("(.)en$", "$1F"), ("(.)eng$", "$1G"), ("(.)ang$", "$1H"), ("ian$", "M"),
      ("(.)an$", "$1J"), ("iao$", "C"), ("(.)ao$", "$1K"), ("(.)ai$", "$1L"),
      ("(.)ei$", "$1Z"), ("ie$", "X"), ("ui$", "V"), ("(.)ou$", "$1B"), ("in$", "N")]
    for (pattern, replacement) in rules {
      value = value.replacingOccurrences(of: pattern, with: replacement, options: .regularExpression)
    }
    return value.lowercased()
  }
}

/// 本地 UI 选择意图，不是 wire DTO；没有 I/O 或持久化，每个意图最多消费一次。
struct InputiaSharedEnglishSelectionState {
  struct Context: Equatable {
    let prefix: String
    let targetID: String
    let cacheIdentity: String
    let clientIdentity: ObjectIdentifier
    let activation: UInt64
  }
  private var pending: (UUID, Context)?
  var hasPending: Bool { pending != nil }
  mutating func begin(_ context: Context) -> UUID? {
    guard pending == nil else { return nil }
    let id = UUID(); pending = (id, context); return id
  }
  func isPending(_ id: UUID) -> Bool { pending?.0 == id }
  mutating func cancel() { pending = nil }
  mutating func consume(_ id: UUID, context: Context, gateAllowed: Bool) -> Bool {
    guard let previous = pending, previous.0 == id else { return false }
    pending = nil
    return gateAllowed && previous.1 == context
  }
}

/// 仅描述原候选索引的完整置换，不拥有候选正文或 Rime 状态。
struct InputiaSharedCandidateOrder: Decodable {
  let ok: Bool
  let mode: String
  let composing: String
  let page: Int
  let indices: [Int]
  static func decode(_ data: Data, mode: String, composing: String, page: Int, count: Int) -> Self? {
    guard count > 0, count <= 9,
      let value = try? JSONDecoder().decode(Self.self, from: data), value.ok,
      value.mode == mode, mode == "Chinese", value.composing == composing, !composing.isEmpty,
      value.page == page, value.indices.count == count,
      value.indices.sorted() == Array(0..<count) else { return nil }
    return value
  }
  func originalIndex(displayed: Int) -> Int? {
    indices.indices.contains(displayed) ? indices[displayed] : nil
  }
  func matches(mode: String, composing: String, page: Int, candidates: [String], originalCandidates: [String]) -> Bool {
    self.mode == mode && self.composing == composing && self.page == page && candidates == originalCandidates
  }
}

/// 只存本进程短租约；所有键盘候选检查只读取此内存与已准备的目标快照。
final class InputiaSharedTermsMemory {
  static let shared = InputiaSharedTermsMemory()
  private let lock = NSLock()
  private var generation: UInt64 = 0
  private var snapshot: InputiaSharedTermsSnapshot?
  var didClear: (() -> Void)?
  func ticket() -> UInt64 { lock.lock(); defer { lock.unlock() }; return generation }
  func clear() {
    lock.lock(); generation &+= 1; snapshot = nil; lock.unlock()
    guard let notify = didClear else { return }
    if Thread.isMainThread { notify() } else { DispatchQueue.main.sync(execute: notify) }
  }
  func install(_ value: InputiaSharedTermsSnapshot, ticket: UInt64, now: TimeInterval = ProcessInfo.processInfo.systemUptime) -> Bool {
    lock.lock(); defer { lock.unlock() }
    guard generation == ticket, now < value.expiresAt else { return false }
    snapshot = value
    return true
  }
  func current(target: InputiaVoiceTarget, now: TimeInterval = ProcessInfo.processInfo.systemUptime) -> InputiaSharedTermsSnapshot? {
    lock.lock(); defer { lock.unlock() }
    guard let snapshot, snapshot.target == target, now < snapshot.expiresAt else { return nil }
    return snapshot
  }
  func expire(identity: String) {
    lock.lock()
    let expired = snapshot.map { $0.identity == identity && ProcessInfo.processInfo.systemUptime >= $0.expiresAt } ?? false
    lock.unlock()
    if expired { clear() }
  }
}

private struct InputiaSharedTermsCommand: Encodable { let lease_id: String; let lease_epoch: UInt64 }
private struct InputiaSharedTermsRequest: Encodable {
  let request_id: String
  let client_instance: String
  let server_instance: String
  let policy_epoch: UInt64
  let shared_terms: InputiaSharedTermsCommand
}
private struct InputiaSharedTermsReply: Decodable {
  let status: String
  let request_id: String
  let lease_id: String?
  let lease_epoch: UInt64?
  let version: InputiaVoiceTermsVersion?
  let terms: [String]?
  let explicit_terms: [String]?
  let max_age_ms: UInt64?
  let code: String?
}
struct InputiaVoicePolicyBarrier: Codable, Equatable {
  let barrier_id: String
  let version: InputiaVoiceTermsVersion
  let clear_shared_personalization: Bool
}
private struct InputiaVoicePolicyAck: Encodable {
  let barrier_id: String
  let version: InputiaVoiceTermsVersion
  let shared_cache_cleared: Bool
  let offline_queue_revalidated: Bool
}

protocol InputiaSharedStateBarrierApplying {
  /// 后台完成内存共享缓存失效、持久快照清理及离线队列重核验；失败必须throw，不能回执成功。
  func applySharedStateBarrier(_ barrier: InputiaVoicePolicyBarrier) throws
}

enum InputiaVoiceServiceError: Error { case profile, handshake, policy }

/// 实际入口使用的候选共享状态。未接入的旧格式/非空队列拒绝确认，绝不丢弃后冒称重核验。
final class InputiaVoiceSharedState: InputiaSharedStateBarrierApplying {
  private var database: OpaquePointer?
  init(profile: InputiaProfile) throws {
    guard !Thread.isMainThread, profile.isCandidate else { throw InputiaVoiceServiceError.profile }
    try profile.validateCandidatePaths()
    try FileManager.default.createDirectory(at: profile.root, withIntermediateDirectories: true, attributes: [.posixPermissions: 0o700])
    guard sqlite3_open_v2(profile.outbox.path, &database, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_NOFOLLOW, nil) == SQLITE_OK else {
      if let database { sqlite3_close(database) }; database = nil
      throw InputiaVoiceServiceError.policy
    }
    do {
      try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: profile.outbox.path)
      sqlite3_busy_timeout(database, 1000)
      let version = try integer("PRAGMA user_version")
      if version == 0 {
        guard try integer("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'") == 0 else { throw InputiaVoiceServiceError.policy }
        try execute("BEGIN IMMEDIATE; CREATE TABLE shared_policy(epoch INTEGER NOT NULL,generation INTEGER NOT NULL); INSERT INTO shared_policy VALUES(0,0); CREATE TABLE shared_terms(term TEXT NOT NULL); CREATE TABLE shared_learning_outbox(event_id TEXT PRIMARY KEY, payload TEXT NOT NULL); PRAGMA user_version=1; COMMIT;")
      } else if version != 1 { throw InputiaVoiceServiceError.policy }
    } catch { sqlite3_close(database); database = nil; throw error }
  }
  deinit { if let database { sqlite3_close(database) } }
  private func execute(_ sql: String) throws {
    guard sqlite3_exec(database, sql, nil, nil, nil) == SQLITE_OK else { throw InputiaVoiceServiceError.policy }
  }
  private func integer(_ sql: String) throws -> Int64 {
    var statement: OpaquePointer?
    guard sqlite3_prepare_v2(database, sql, -1, &statement, nil) == SQLITE_OK else { throw InputiaVoiceServiceError.policy }
    defer { sqlite3_finalize(statement) }
    guard sqlite3_step(statement) == SQLITE_ROW else { throw InputiaVoiceServiceError.policy }
    return sqlite3_column_int64(statement, 0)
  }
  func lastVersion() throws -> InputiaVoiceTermsVersion {
    let epoch = try integer("SELECT epoch FROM shared_policy"), generation = try integer("SELECT generation FROM shared_policy")
    guard epoch >= 0, generation >= 0 else { throw InputiaVoiceServiceError.policy }
    return InputiaVoiceTermsVersion(policy_epoch: UInt64(epoch), learning_generation: UInt64(generation))
  }
  func applySharedStateBarrier(_ barrier: InputiaVoicePolicyBarrier) throws {
    guard !Thread.isMainThread, barrier.version.policy_epoch <= UInt64(Int64.max),
          barrier.version.learning_generation <= UInt64(Int64.max) else { throw InputiaVoiceServiceError.policy }
    try execute("BEGIN IMMEDIATE")
    do {
      let old = try lastVersion()
      guard barrier.version.policy_epoch >= old.policy_epoch,
            barrier.version.learning_generation >= old.learning_generation,
            try integer("SELECT COUNT(*) FROM shared_learning_outbox") == 0 else { throw InputiaVoiceServiceError.policy }
      try execute("DELETE FROM shared_terms; UPDATE shared_policy SET epoch=\(barrier.version.policy_epoch),generation=\(barrier.version.learning_generation); COMMIT;")
    } catch { try? execute("ROLLBACK"); throw error }
  }
}

struct InputiaTargetBridgeCommand: Encodable {
  let kind: String
  var draft: InputiaVoiceTarget? = nil
  var target: InputiaVoiceTarget? = nil
  var purpose: String? = nil
  var operation_id: String? = nil
  var target_id: String? = nil
}
struct InputiaTargetBridgeSelection: Decodable { let location: Int; let length: Int }
struct InputiaTargetBridgeReply: Decodable {
  let status: String
  let request_id: String
  let ready: Bool
  let server_instance: String
  let permission_epoch: UInt64
  let valid_for_ms: UInt64
  let target: InputiaVoiceTarget?
  let selection: InputiaTargetBridgeSelection?
  let dispatch_nonce: String?
  let code: String?
  var deadline: TimeInterval = 0
  private enum CodingKeys: String, CodingKey {
    case status, request_id, ready, server_instance, permission_epoch, valid_for_ms, target, selection, dispatch_nonce, code
  }
}
private struct InputiaTargetBridgeRequest: Encodable {
  let request_id: String
  let client_instance: String
  let server_instance: String
  let policy_epoch: UInt64
  let target_bridge: InputiaTargetBridgeCommand
}

struct InputiaTypedCaptureCommand: Encodable {
  let kind: String
  var capture_epoch: UInt64? = nil
  var event_id: String? = nil
  var segment_id: String? = nil
  var text: String? = nil
  var draft: InputiaVoiceTarget? = nil
}
struct InputiaTypedCaptureReply: Decodable {
  let status: String
  let request_id: String
  let server_instance: String
  let enabled: Bool
  let epoch: UInt64
  let saved: Bool
  let code: String?
}
private struct InputiaTypedCaptureRequest: Encodable {
  let request_id: String
  let client_instance: String
  let server_instance: String
  let policy_epoch: UInt64
  let typed_capture: InputiaTypedCaptureCommand
}

struct InputiaPersonalCandidate: Codable, Equatable {
  let id: String
  let text: String
  let base_rank: Int
  let consumed_len: Int
  var match_type: String? = nil
}
struct InputiaPersonalPrediction: Codable, Equatable { let id: String; let text: String }
struct InputiaPersonalResult: Decodable {
  let ordered_ids: [String]?
  let recalled_candidates: [InputiaPersonalCandidate]?
  let predictions: [InputiaPersonalPrediction]?
  let context_id: String?
  let admitted: Bool?
  let prediction_id: String?
}
struct InputiaPersonalCommand: Encodable {
  let kind: String
  var target: InputiaVoiceTarget? = nil
  var learning_epoch: UInt64? = nil
  var input_code: String? = nil
  var schema_id: String? = nil
  var context: String? = nil
  var context_id: String? = nil
  var candidates: [InputiaPersonalCandidate]? = nil
  var limit: Int? = nil
  var event_id: String? = nil
  var text: String? = nil
  var previous: String? = nil
  var explicit_selection: Bool? = nil
  var original_rank: Int? = nil
  var operation: String? = nil
  var prediction_id: String? = nil
}
struct InputiaPersonalReply: Decodable {
  let privacy_barrier: InputiaVoicePolicyBarrier?
  let status: String
  let request_id: String
  let server_instance: String
  let enabled: Bool
  let epoch: UInt64
  let result: InputiaPersonalResult?
  let code: String?
}
private struct InputiaPersonalRequest: Encodable {
  let request_id: String
  let client_instance: String
  let server_instance: String
  let policy_epoch: UInt64
  let personalization: InputiaPersonalCommand
}

struct InputiaVoiceTarget: Codable, Equatable {
  let target_id: String
  let host_instance: String
  let controller_id: String
  let activation_generation: UInt64
  let field_id: String?
  let selection_generation: UInt64
  let composition_generation: UInt64
  let source_app: String?
}

enum InputiaVoiceCommand: Encodable {
  case start(target: InputiaVoiceTarget, postProcess: Bool, terms: InputiaVoiceTermsVersion)
  case hostShortcut(target: InputiaVoiceTarget, postProcess: Bool, terms: InputiaVoiceTermsVersion, edge: InputiaHostShortcutEdge)
  case stop, cancel, status
  private enum Keys: String, CodingKey { case kind, target, post_process, terms, edge }
  func encode(to encoder: Encoder) throws {
    var values = encoder.container(keyedBy: Keys.self)
    switch self {
    case .start(let target, let postProcess, let terms):
      try values.encode("start", forKey: .kind)
      try values.encode(target, forKey: .target)
      try values.encode(postProcess, forKey: .post_process)
      try values.encode(terms, forKey: .terms)
    case .hostShortcut(let target, let postProcess, let terms, let edge):
      try values.encode("host_shortcut", forKey: .kind)
      try values.encode(target, forKey: .target)
      try values.encode(postProcess, forKey: .post_process)
      try values.encode(terms, forKey: .terms)
      try values.encode(edge, forKey: .edge)
    case .stop: try values.encode("stop", forKey: .kind)
    case .cancel: try values.encode("cancel", forKey: .kind)
    case .status: try values.encode("status", forKey: .kind)
    }
  }
}

enum InputiaVoiceShortcutActivation: String, Codable {
  case toggle
  case pushToTalk = "push_to_talk"
  case holdOrToggle = "hold_or_toggle"
}

struct InputiaHostShortcutEdge: Codable {
  let trigger_id: String
  let starts_session: Bool
  let lease_id: String
  let lease_epoch: UInt64
  let binding_id: String
  let hotkey_string: String
  let is_pressed: Bool
  let activation: InputiaVoiceShortcutActivation
  let pressed_at_unix_ms: UInt64
  let hold_threshold_ms: UInt64
}

struct InputiaHostShortcutLease: Codable {
  let lease_id: String
  let lease_epoch: UInt64
  let target: InputiaVoiceTarget
  let issued_at_unix_ms: UInt64
  let expires_at_unix_ms: UInt64
}

struct InputiaHostShortcutTrigger: Decodable {
  let trigger_id: String
  let session_id: String
  let starts_session: Bool
  let lease_id: String
  let lease_epoch: UInt64
  let target: InputiaVoiceTarget
  let binding_id: String
  let hotkey_string: String
  let is_pressed: Bool
  let activation: InputiaVoiceShortcutActivation
  let pressed_at_unix_ms: UInt64
  let hold_threshold_ms: UInt64
  let server_instance: String
  let client_instance: String
  let policy_epoch: UInt64

  var edge: InputiaHostShortcutEdge {
    InputiaHostShortcutEdge(trigger_id: trigger_id, starts_session: starts_session, lease_id: lease_id, lease_epoch: lease_epoch,
      binding_id: binding_id, hotkey_string: hotkey_string, is_pressed: is_pressed,
      activation: activation, pressed_at_unix_ms: pressed_at_unix_ms, hold_threshold_ms: hold_threshold_ms)
  }
}

private struct InputiaHostShortcutCommand: Encodable {
  let kind: String
  var lease: InputiaHostShortcutLease? = nil
  var max_wait_ms: UInt64? = nil
  var lease_id: String? = nil
  var lease_epoch: UInt64? = nil
  var trigger_id: String? = nil
}
private struct InputiaHostShortcutRequest: Encodable {
  let request_id: String
  let client_instance: String
  let server_instance: String
  let policy_epoch: UInt64
  let shortcut: InputiaHostShortcutCommand
}
private struct InputiaHostShortcutReply: Decodable {
  let status: String
  let request_id: String
  let lease_id: String?
  let lease_epoch: UInt64?
  let trigger: InputiaHostShortcutTrigger?
  let code: String?
}
struct InputiaVoiceRequest: Encodable {
  let request_id: String
  let session_id: String
  let server_instance: String
  let client_instance: String
  let policy_epoch: UInt64
  let command: InputiaVoiceCommand
}
struct InputiaVoiceSessionView: Codable {
  let session_id: String
  let generation: UInt64
  let phase: String
  let target_id: String?
  let item_id: String?
  let output_operation_id: String?
}
struct InputiaVoiceReply: Decodable {
  let status: String
  let request_id: String
  let view: InputiaVoiceSessionView?
  let code: String?
}

/// 正文只在已认证连接与原目标回调之间短暂存在，不写诊断或重放队列。
struct InputiaVoiceDelivery: Codable {
  let operation_id: String
  let session_id: String
  let item_id: String
  let revision: UInt64
  let policy_epoch: UInt64
  let target_id: String
  let text: String
  var dispatchDeadline: TimeInterval = 0
  var dispatchNonce: String? = nil
  private enum CodingKeys: String, CodingKey {
    case operation_id, session_id, item_id, revision, policy_epoch, target_id, text
  }
}

private struct InputiaVoiceOutputCommand: Encodable {
  let kind: String
  var operation_id: String? = nil
  var receipt: String? = nil
}
private struct InputiaVoiceOutputRequest: Encodable {
  let request_id: String
  let session_id: String
  let server_instance: String
  let client_instance: String
  let policy_epoch: UInt64
  let output: InputiaVoiceOutputCommand
}
private struct InputiaVoiceOutputReply: Decodable {
  let status: String
  let request_id: String
  let delivery: InputiaVoiceDelivery?
  let operation_id: String?
  let state: String?
  let code: String?
}

/// 单一后台队列拥有此客户端。构建公钥作为信任根，manifest只提供被签名的两端身份。
final class InputiaVoiceServiceConnection {
  static let processInstance = UUID().uuidString
  private let connection: InputiaFramedConnection
  private var privacyState: InputiaSharedStateBarrierApplying?
  private static var appliedPrivacyEpoch: UInt64 = 0
  private static var appliedPrivacyServer: String?
  let server: InputiaVoiceHello
  private(set) var locallyAppliedVersion: InputiaVoiceTermsVersion?
  static func sharedTermsConnectionMatches(server: String, primaryServer: String,
                                          version: InputiaVoiceTermsVersion?, primaryVersion: InputiaVoiceTermsVersion?) -> Bool {
    guard let version, let primaryVersion else { return false }
    // 词库代际以取词连接已 ACK 的真实 barrier 为准，热键连接只拥有策略 epoch。
    return server == primaryServer && version.policy_epoch == primaryVersion.policy_epoch
  }

  private init(connection: InputiaFramedConnection, server: InputiaVoiceHello) {
    self.connection = connection
    self.server = server
  }

  static func connect(endpoint: String, signedManifest: Data, trust: PairTrust,
                      profile: InputiaProfile, previousEpoch: UInt64) throws -> InputiaVoiceServiceConnection {
    guard !Thread.isMainThread else { throw InputiaConnectionError.mainThread }
    guard profile.isCandidate, profile.runID == trust.runID,
          trust.profileID == "unified-candidate:\(trust.runID)",
          trust.localRole == .inputia, trust.requireHardenedRuntime else { throw InputiaVoiceServiceError.profile }
    let manifest = try SignedPairManifest.verify(signedManifest, trust: trust)
    let transport = try InputiaFramedConnection.connect(path: endpoint)
    do {
      _ = try transport.authenticate { descriptor in
        try PeerAuthenticator.authenticate(socketFD: descriptor, manifest: manifest, expectedRole: .handy)
      }
      let hello = InputiaVoiceHello(protocol_major: 1, protocol_minor: 0, instance_id: processInstance,
        profile_id: trust.profileID, policy_epoch: previousEpoch, capabilities: ["voice_sessions_v1", "shared_terms_v1", "ime_target_broker_v1", "typed_capture_v1", "personalization_v1"])
      try transport.write(hello)
      let reply = try transport.read(InputiaVoiceHelloReply.self)
      guard reply.status == "accepted", let server = reply.server,
            server.protocol_major == 1, reply.negotiated_minor == 0,
            server.profile_id == trust.profileID, !server.instance_id.isEmpty,
            server.pair_binding == nil,
            server.instance_id.utf8.count <= 256,
            !server.instance_id.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
            server.policy_epoch >= previousEpoch,
            reply.require_policy_refresh == (server.policy_epoch != previousEpoch),
            server.capabilities.contains("voice_sessions_v1") else { throw InputiaVoiceServiceError.handshake }
      return InputiaVoiceServiceConnection(connection: transport, server: server)
    } catch { transport.close(); throw error }
  }

  /// v2 使用安装收据绑定运行域，失败不进入旧 profile 握手。
  static func connect(endpoint: String, signedManifest: Data, trust: PairReleaseTrust,
                      profile: InputiaProfile, previousEpoch: UInt64) throws -> InputiaVoiceServiceConnection {
    guard !Thread.isMainThread else { throw InputiaConnectionError.mainThread }
    guard let installation = profile.installation, let binding = profile.pairBinding,
      binding.product_id == trust.productID, binding.pair_release_id == trust.releaseID,
      trust.localRole == .inputia, trust.requireHardenedRuntime else { throw InputiaVoiceServiceError.profile }
    let manifest = try SignedReleasePairManifest.verify(signedManifest, trust: trust)
    let transport = try InputiaFramedConnection.connect(path: endpoint)
    do {
      _ = try transport.authenticate { descriptor in
        try PeerAuthenticator.authenticate(socketFD: descriptor, manifest: manifest, expectedRole: .handy,
          expectedBundlePath: installation.receipt.components.control)
      }
      let hello = InputiaVoiceHello(protocol_major: 1, protocol_minor: 1, instance_id: processInstance,
        profile_id: profile.profileID, policy_epoch: previousEpoch,
        capabilities: ["voice_sessions_v1", "shared_terms_v1", "ime_target_broker_v1", "typed_capture_v1", "personalization_v1", "installation_binding_v1"],
        pair_binding: binding)
      try transport.write(hello)
      let reply = try transport.read(InputiaVoiceHelloReply.self)
      guard reply.status == "accepted", let server = reply.server,
        server.protocol_major == 1, server.protocol_minor >= 1, reply.negotiated_minor == 1,
        server.profile_id == profile.profileID, server.pair_binding == binding,
        !server.instance_id.isEmpty, server.instance_id.utf8.count <= 256,
        !server.instance_id.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
        server.policy_epoch >= previousEpoch,
        reply.require_policy_refresh == (server.policy_epoch != previousEpoch),
        server.capabilities.contains("voice_sessions_v1"),
        server.capabilities.contains("installation_binding_v1") else { throw InputiaVoiceServiceError.handshake }
      return InputiaVoiceServiceConnection(connection: transport, server: server)
    } catch { transport.close(); throw error }
  }

  func synchronizePolicy(using state: InputiaSharedStateBarrierApplying) throws {
    privacyState = state
    InputiaSharedTermsMemory.shared.clear()
    locallyAppliedVersion = nil
    do {
      let barrier = try connection.read(InputiaVoicePolicyBarrier.self)
      try Self.applyAndAcknowledge(barrier, minimumEpoch: server.policy_epoch, state: state, privacyServer: server.instance_id) { ack in
        try connection.write(ack)
      }
      // 仅说明本地应用并发送了ACK，不冒称服务已接纳Start或麦克风正在录音。
      locallyAppliedVersion = barrier.version
    } catch { connection.close(); throw error }
  }

  private static func applyAndAcknowledge(_ barrier: InputiaVoicePolicyBarrier, minimumEpoch: UInt64,
                                          state: InputiaSharedStateBarrierApplying, privacyServer: String? = nil,
                                          send: (InputiaVoicePolicyAck) throws -> Void) throws {
    guard barrier.clear_shared_personalization, barrier.version.policy_epoch >= minimumEpoch,
          barrier.barrier_id.utf8.count == 64,
          barrier.barrier_id.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
      throw InputiaVoiceServiceError.policy
    }
    InputiaSharedTermsMemory.shared.clear()
    try state.applySharedStateBarrier(barrier)
    // 主线程清理已呈现的候选、短上下文和在途票据后才能 ACK；后台不等待键入逻辑持锁。
    let clear = {
      if appliedPrivacyServer != privacyServer || appliedPrivacyEpoch < barrier.version.policy_epoch {
        appliedPrivacyEpoch = barrier.version.policy_epoch; appliedPrivacyServer = privacyServer
        NotificationCenter.default.post(name: Notification.Name("InputiaPrivacyRevoked"), object: nil)
      }
    }
    if Thread.isMainThread { clear() } else { DispatchQueue.main.sync(execute: clear) }
    try send(InputiaVoicePolicyAck(barrier_id: barrier.barrier_id, version: barrier.version,
      shared_cache_cleared: true, offline_queue_revalidated: true))
  }

  func close() {
    locallyAppliedVersion = nil; connection.close()
    InputiaSharedTermsMemory.shared.clear()
  }

  /// 短期键入连接不拥有全局词库缓存，不影响现有候选租约。
  func closeTypedCaptureConnection() { locallyAppliedVersion = nil; connection.close() }

  func personalization(_ command: InputiaPersonalCommand) throws -> InputiaPersonalReply {
    guard !Thread.isMainThread, let version = locallyAppliedVersion,
      server.capabilities.contains("personalization_v1") else { throw InputiaVoiceServiceError.policy }
    if let target = command.target, target.host_instance != Self.processInstance { throw InputiaVoiceServiceError.policy }
    let id = UUID().uuidString
    try connection.write(InputiaPersonalRequest(request_id: id, client_instance: Self.processInstance,
      server_instance: server.instance_id, policy_epoch: version.policy_epoch, personalization: command))
    let reply = try connection.read(InputiaPersonalReply.self)
    guard reply.status == "personalization", reply.request_id == id,
      reply.server_instance == server.instance_id else { throw InputiaVoiceServiceError.handshake }
    if let barrier = reply.privacy_barrier {
      guard let privacyState else { throw InputiaVoiceServiceError.policy }
      try Self.applyAndAcknowledge(barrier, minimumEpoch: version.policy_epoch, state: privacyState, privacyServer: server.instance_id) { ack in try connection.write(ack) }
      locallyAppliedVersion = barrier.version
    }
    return reply
  }

  func typedCapture(_ command: InputiaTypedCaptureCommand) throws -> InputiaTypedCaptureReply {
    guard !Thread.isMainThread, let version = locallyAppliedVersion,
      server.capabilities.contains("typed_capture_v1") else { throw InputiaVoiceServiceError.policy }
    if let draft = command.draft, draft.host_instance != Self.processInstance { throw InputiaVoiceServiceError.policy }
    let id = UUID().uuidString
    try connection.write(InputiaTypedCaptureRequest(request_id: id, client_instance: Self.processInstance,
      server_instance: server.instance_id, policy_epoch: version.policy_epoch, typed_capture: command))
    let reply = try connection.read(InputiaTypedCaptureReply.self)
    guard reply.status == "typed_capture", reply.request_id == id,
      reply.server_instance == server.instance_id else { throw InputiaVoiceServiceError.handshake }
    return reply
  }

  func targetBridge(_ command: InputiaTargetBridgeCommand) throws -> InputiaTargetBridgeReply {
    guard !Thread.isMainThread, let version = locallyAppliedVersion,
      server.capabilities.contains("ime_target_broker_v1") else { throw InputiaVoiceServiceError.policy }
    let requestID = UUID().uuidString
    let started = ProcessInfo.processInfo.systemUptime
    do {
      try connection.write(InputiaTargetBridgeRequest(request_id: requestID, client_instance: Self.processInstance,
        server_instance: server.instance_id, policy_epoch: version.policy_epoch, target_bridge: command))
      var reply = try connection.read(InputiaTargetBridgeReply.self)
      let maximumAge: UInt64 = command.kind == "capture" ? 120_000 : (command.kind == "status" ? 1_000 : 250)
      guard reply.status == "target_bridge", reply.request_id == requestID,
        reply.server_instance == server.instance_id, reply.valid_for_ms <= maximumAge else { throw InputiaVoiceServiceError.handshake }
      reply.deadline = started + Double(reply.valid_for_ms) / 1000
      return reply
    } catch { close(); throw error }
  }

  func fetchSharedTerms(lease: InputiaHostShortcutLease, leaseDeadline: TimeInterval) throws -> InputiaSharedTermsSnapshot? {
    guard server.capabilities.contains("shared_terms_v1") else { return nil }
    do {
      guard !Thread.isMainThread, let applied = locallyAppliedVersion,
        lease.target.host_instance == Self.processInstance else { throw InputiaVoiceServiceError.policy }
      let sentAt = ProcessInfo.processInfo.systemUptime
      let requestID = UUID().uuidString
      try connection.write(InputiaSharedTermsRequest(request_id: requestID,
        client_instance: Self.processInstance, server_instance: server.instance_id,
        policy_epoch: applied.policy_epoch,
        shared_terms: InputiaSharedTermsCommand(lease_id: lease.lease_id, lease_epoch: lease.lease_epoch)))
      let reply = try connection.read(InputiaSharedTermsReply.self)
      guard reply.request_id == requestID else { throw InputiaVoiceServiceError.handshake }
      if reply.status == "rejected", reply.code != nil {
        InputiaSharedTermsMemory.shared.clear(); return nil
      }
      guard reply.status == "shared_terms", reply.code == nil,
        reply.lease_id == lease.lease_id, reply.lease_epoch == lease.lease_epoch,
        let version = reply.version, version == applied,
        let terms = reply.terms, terms.count <= 256,
        terms.reduce(0, { $0 + $1.utf8.count }) <= 16 * 1024,
        terms.allSatisfy({ !$0.isEmpty && $0.count <= 32 && !$0.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) }),
        let maxAge = reply.max_age_ms, maxAge > 0, maxAge <= 1000
      else { throw InputiaVoiceServiceError.handshake }
      let explicit = reply.explicit_terms ?? []
      guard explicit.count <= 256, explicit.reduce(0, { $0 + $1.utf8.count }) <= 16 * 1024,
        explicit.allSatisfy({ !$0.isEmpty && $0.count <= 128 && !$0.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) })
      else { throw InputiaVoiceServiceError.handshake }
      let deadline = min(sentAt + Double(maxAge) / 1000, leaseDeadline)
      guard ProcessInfo.processInfo.systemUptime < deadline else {
        InputiaSharedTermsMemory.shared.clear(); return nil
      }
      return InputiaSharedTermsSnapshot(identity: "\(server.instance_id):\(requestID):\(lease.lease_id):\(lease.lease_epoch)",
        target: lease.target, version: version, terms: terms, explicitTerms: explicit, expiresAt: deadline)
    } catch { close(); throw error }
  }

  func registerShortcutLease(_ lease: InputiaHostShortcutLease) throws {
    guard lease.target.host_instance == Self.processInstance else { throw InputiaVoiceServiceError.handshake }
    let reply = try shortcutRequest(InputiaHostShortcutCommand(kind: "register", lease: lease))
    guard reply.status == "registered", reply.lease_id == lease.lease_id,
      reply.lease_epoch == lease.lease_epoch else { close(); throw InputiaVoiceServiceError.handshake }
  }

  func pollShortcut(maxWaitMs: UInt64 = 250) throws -> InputiaHostShortcutTrigger? {
    guard maxWaitMs <= 1000 else { throw InputiaVoiceServiceError.handshake }
    let reply = try shortcutRequest(InputiaHostShortcutCommand(kind: "poll", max_wait_ms: maxWaitMs))
    if reply.status == "empty", reply.trigger == nil { return nil }
    guard reply.status == "trigger", let trigger = reply.trigger,
      trigger.client_instance == Self.processInstance, trigger.server_instance == server.instance_id,
      let applied = locallyAppliedVersion,
      trigger.starts_session ? trigger.policy_epoch == applied.policy_epoch : trigger.policy_epoch <= applied.policy_epoch,
      trigger.target.host_instance == Self.processInstance,
      ["transcribe", "transcribe_with_post_process"].contains(trigger.binding_id),
      !trigger.trigger_id.isEmpty, trigger.trigger_id.utf8.count <= 256,
      !trigger.session_id.isEmpty, trigger.session_id.utf8.count <= 256,
      trigger.hotkey_string.utf8.count <= 256
    else { close(); throw InputiaVoiceServiceError.handshake }
    return trigger
  }

  func retireShortcutLease(_ lease: InputiaHostShortcutLease) throws {
    let reply = try shortcutRequest(InputiaHostShortcutCommand(kind: "retire", lease_id: lease.lease_id, lease_epoch: lease.lease_epoch))
    guard reply.status == "retired" else { close(); throw InputiaVoiceServiceError.handshake }
  }

  func rejectUnconsumedShortcut(_ triggerID: String) throws -> Bool {
    let reply = try shortcutRequest(InputiaHostShortcutCommand(kind: "reject", trigger_id: triggerID))
    if reply.status == "retired" { return true }
    guard reply.status == "rejected", reply.code != nil else { close(); throw InputiaVoiceServiceError.handshake }
    return false
  }

  private func shortcutRequest(_ command: InputiaHostShortcutCommand) throws -> InputiaHostShortcutReply {
    do {
      guard !Thread.isMainThread, let version = locallyAppliedVersion else { throw InputiaVoiceServiceError.policy }
      let requestID = UUID().uuidString
      try connection.write(InputiaHostShortcutRequest(request_id: requestID,
        client_instance: Self.processInstance, server_instance: server.instance_id,
        policy_epoch: version.policy_epoch, shortcut: command))
      let reply = try connection.read(InputiaHostShortcutReply.self)
      guard reply.request_id == requestID else { throw InputiaVoiceServiceError.handshake }
      if command.kind == "reject", reply.status == "rejected",
        let code = reply.code, ["unauthorized", "unknown", "coordinator_rejected"].contains(code) {
        return reply
      }
      guard reply.code == nil else { throw InputiaVoiceServiceError.handshake }
      return reply
    } catch { close(); throw error }
  }

  func menuRequest(kind: String, modelID: String? = nil) throws -> InputiaMenuReply {
    do {
      guard !Thread.isMainThread, let version = locallyAppliedVersion,
        ["status", "copy_latest", "history", "settings", "check_updates", "unload_model", "select_model", "quit_service"].contains(kind)
      else { throw InputiaVoiceServiceError.policy }
      let requestID = UUID().uuidString
      try connection.write(InputiaMenuRequest(request_id: requestID, client_instance: Self.processInstance,
        server_instance: server.instance_id, policy_epoch: version.policy_epoch,
        menu: InputiaMenuCommand(kind: kind, model_id: modelID)))
      let reply = try connection.read(InputiaMenuReply.self)
      guard reply.request_id == requestID else { throw InputiaVoiceServiceError.handshake }
      if reply.status == "rejected" {
        guard let code = reply.code, ["unauthorized", "missing_session", "unknown", "coordinator_rejected"].contains(code) else {
          throw InputiaVoiceServiceError.handshake
        }
        // 已完整收到的业务拒绝不会破坏正在使用同一连接的语音会话。
        return reply
      }
      guard reply.status == "menu", reply.selected_model != nil,
        let models = reply.models, models.count <= 1024, reply.busy != nil,
        models.allSatisfy({ !$0.id.isEmpty && $0.id.utf8.count <= 256 && $0.name.utf8.count <= 1024 })
      else { throw InputiaVoiceServiceError.handshake }
      return reply
    } catch { close(); throw error }
  }

  /// 服务端在返回正文前持久claim；此方法绝不重试fetch，也不跨连接恢复正文。
  func fetchDelivery(view: InputiaVoiceSessionView, target: InputiaVoiceTarget) throws -> InputiaVoiceDelivery? {
    let deadline = ProcessInfo.processInfo.systemUptime + 2
    let reply = try outputRequest(sessionID: view.session_id, output: InputiaVoiceOutputCommand(kind: "fetch"))
    if reply.status == "output" { return nil }
    guard reply.status == "delivery", var delivery = reply.delivery else {
      close(); throw InputiaVoiceServiceError.handshake
    }
    do { try Self.validateDelivery(delivery, view: view, target: target, epoch: locallyAppliedVersion?.policy_epoch) }
    catch { close(); throw error }
    delivery.dispatchDeadline = deadline
    return delivery
  }

  static func validateDelivery(_ delivery: InputiaVoiceDelivery, view: InputiaVoiceSessionView,
                               target: InputiaVoiceTarget, epoch: UInt64?) throws {
    guard delivery.session_id == view.session_id,
          delivery.operation_id == view.output_operation_id,
          delivery.item_id == view.item_id, delivery.target_id == target.target_id,
          delivery.policy_epoch == epoch,
          !delivery.text.isEmpty, delivery.text.utf8.count <= 192 * 1024 else {
      throw InputiaVoiceServiceError.handshake
    }
  }

  func acknowledgeDelivery(_ delivery: InputiaVoiceDelivery, receipt: String) throws {
    guard ["dispatched", "pending_target", "uncertain"].contains(receipt) else {
      close(); throw InputiaVoiceServiceError.handshake
    }
    let reply = try outputRequest(sessionID: delivery.session_id,
      output: InputiaVoiceOutputCommand(kind: "receipt", operation_id: delivery.operation_id, receipt: receipt))
    let expected = receipt == "dispatched" ? "dispatched_only" : receipt
    guard reply.status == "output", reply.operation_id == delivery.operation_id, reply.state == expected else {
      close(); throw InputiaVoiceServiceError.handshake
    }
  }

  private func outputRequest(sessionID: String, output: InputiaVoiceOutputCommand) throws -> InputiaVoiceOutputReply {
    do {
      guard !Thread.isMainThread, let version = locallyAppliedVersion else { throw InputiaVoiceServiceError.policy }
      let requestID = UUID().uuidString
      try connection.write(InputiaVoiceOutputRequest(request_id: requestID, session_id: sessionID,
        server_instance: server.instance_id, client_instance: Self.processInstance,
        policy_epoch: version.policy_epoch, output: output))
      let reply = try connection.read(InputiaVoiceOutputReply.self)
      guard reply.request_id == requestID else { throw InputiaVoiceServiceError.handshake }
      switch reply.status {
      case "delivery":
        guard output.kind == "fetch", reply.delivery != nil, reply.state == nil, reply.code == nil else { throw InputiaVoiceServiceError.handshake }
      case "output":
        guard reply.delivery == nil, let state = reply.state, reply.operation_id != nil, reply.code == nil,
              ["prepared", "dispatched", "confirmed", "dispatched_only", "uncertain", "pending_target", "rejected"].contains(state) else { throw InputiaVoiceServiceError.handshake }
      default: throw InputiaVoiceServiceError.handshake
      }
      return reply
    } catch { close(); throw error }
  }

  /// 单次请求只写一次，读回执失败不重放Start/输出。调用者以同session的Status查询事实。
  func request(sessionID: String, requestID: String, command: InputiaVoiceCommand) throws -> InputiaVoiceReply {
    do {
      guard !sessionID.isEmpty, sessionID.utf8.count <= 256,
            !requestID.isEmpty, requestID.utf8.count <= 256,
            !sessionID.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
            !requestID.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
        throw InputiaVoiceServiceError.handshake
      }
      var requestEpoch = locallyAppliedVersion?.policy_epoch ?? server.policy_epoch
      switch command {
      case .start(let target, _, let terms):
        guard target.host_instance == Self.processInstance, terms == locallyAppliedVersion else {
          throw InputiaVoiceServiceError.policy
        }
      case .hostShortcut(let target, _, let terms, let edge):
        guard target.host_instance == Self.processInstance, let applied = locallyAppliedVersion,
          edge.starts_session ? terms == applied : terms.policy_epoch <= applied.policy_epoch
        else { throw InputiaVoiceServiceError.policy }
        requestEpoch = terms.policy_epoch
      default: break
      }
      let request = InputiaVoiceRequest(request_id: requestID, session_id: sessionID,
        server_instance: server.instance_id, client_instance: Self.processInstance,
        policy_epoch: requestEpoch, command: command)
      try connection.write(request)
      let reply = try connection.read(InputiaVoiceReply.self)
      guard reply.request_id == requestID else { throw InputiaVoiceServiceError.handshake }
      if reply.status == "session" {
        guard let view = reply.view, view.session_id == sessionID,
              ["preparing", "recording", "processing", "pending_target", "dispatched", "confirmed", "uncertain", "cancelled", "failed", "interrupted"].contains(view.phase),
              reply.code == nil else { throw InputiaVoiceServiceError.handshake }
      } else {
        guard reply.status == "rejected", reply.view == nil, let code = reply.code,
              ["unauthorized", "missing_session", "unknown", "coordinator_rejected"].contains(code) else {
          throw InputiaVoiceServiceError.handshake
        }
      }
      return reply
    } catch { close(); throw error }
  }

  #if INPUTIA_CONNECTION_SELF_CHECK
  static func fixture(descriptor: Int32, server: InputiaVoiceHello, version: InputiaVoiceTermsVersion) throws -> InputiaVoiceServiceConnection {
    let value = InputiaVoiceServiceConnection(connection: try InputiaFramedConnection.fixture(descriptor: descriptor, timeout: 2), server: server)
    value.locallyAppliedVersion = version
    return value
  }
  static func checkPolicy(_ barrier: InputiaVoicePolicyBarrier, minimumEpoch: UInt64,
                          state: InputiaSharedStateBarrierApplying, sent: () -> Void) throws {
    try applyAndAcknowledge(barrier, minimumEpoch: minimumEpoch, state: state) { _ in sent() }
  }
  #endif
}
#endif
