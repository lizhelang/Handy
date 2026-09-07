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
  case stop, cancel, status
  private enum Keys: String, CodingKey { case kind, target, post_process, terms }
  func encode(to encoder: Encoder) throws {
    var values = encoder.container(keyedBy: Keys.self)
    switch self {
    case .start(let target, let postProcess, let terms):
      try values.encode("start", forKey: .kind)
      try values.encode(target, forKey: .target)
      try values.encode(postProcess, forKey: .post_process)
      try values.encode(terms, forKey: .terms)
    case .stop: try values.encode("stop", forKey: .kind)
    case .cancel: try values.encode("cancel", forKey: .kind)
    case .status: try values.encode("status", forKey: .kind)
    }
  }
}
struct InputiaVoiceRequest: Encodable {
  let request_id: String
  let session_id: String
  let server_instance: String
  let client_instance: String
  let policy_epoch: UInt64
  let command: InputiaVoiceCommand
}
struct InputiaVoiceSessionView: Decodable {
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

/// 单一后台队列拥有此客户端。构建公钥作为信任根，manifest只提供被签名的两端身份。
final class InputiaVoiceServiceConnection {
  static let processInstance = UUID().uuidString
  private let connection: InputiaFramedConnection
  let server: InputiaVoiceHello
  private(set) var locallyAppliedVersion: InputiaVoiceTermsVersion?

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
        profile_id: trust.profileID, policy_epoch: previousEpoch, capabilities: ["voice_sessions_v1"])
      try transport.write(hello)
      let reply = try transport.read(InputiaVoiceHelloReply.self)
      guard reply.status == "accepted", let server = reply.server,
            server.protocol_major == 1, reply.negotiated_minor == 0,
            server.profile_id == trust.profileID, !server.instance_id.isEmpty,
            server.instance_id.utf8.count <= 256,
            !server.instance_id.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
            server.policy_epoch >= previousEpoch,
            reply.require_policy_refresh == (server.policy_epoch != previousEpoch),
            server.capabilities.contains("voice_sessions_v1") else { throw InputiaVoiceServiceError.handshake }
      return InputiaVoiceServiceConnection(connection: transport, server: server)
    } catch { transport.close(); throw error }
  }

  func synchronizePolicy(using state: InputiaSharedStateBarrierApplying) throws {
    locallyAppliedVersion = nil
    do {
      let barrier = try connection.read(InputiaVoicePolicyBarrier.self)
      try Self.applyAndAcknowledge(barrier, minimumEpoch: server.policy_epoch, state: state) { ack in
        try connection.write(ack)
      }
      // 仅说明本地应用并发送了ACK，不冒称服务已接纳Start或麦克风正在录音。
      locallyAppliedVersion = barrier.version
    } catch { connection.close(); throw error }
  }

  private static func applyAndAcknowledge(_ barrier: InputiaVoicePolicyBarrier, minimumEpoch: UInt64,
                                          state: InputiaSharedStateBarrierApplying,
                                          send: (InputiaVoicePolicyAck) throws -> Void) throws {
    guard barrier.clear_shared_personalization, barrier.version.policy_epoch >= minimumEpoch,
          barrier.barrier_id.utf8.count == 64,
          barrier.barrier_id.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }) else {
      throw InputiaVoiceServiceError.policy
    }
    try state.applySharedStateBarrier(barrier)
    try send(InputiaVoicePolicyAck(barrier_id: barrier.barrier_id, version: barrier.version,
      shared_cache_cleared: true, offline_queue_revalidated: true))
  }

  func close() { locallyAppliedVersion = nil; connection.close() }

  /// 单次请求只写一次，读回执失败不重放Start/输出。调用者以同session的Status查询事实。
  func request(sessionID: String, requestID: String, command: InputiaVoiceCommand) throws -> InputiaVoiceReply {
    do {
      guard !sessionID.isEmpty, sessionID.utf8.count <= 256,
            !requestID.isEmpty, requestID.utf8.count <= 256,
            !sessionID.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
            !requestID.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }) else {
        throw InputiaVoiceServiceError.handshake
      }
      if case .start(let target, _, let terms) = command {
        guard target.host_instance == Self.processInstance, terms == locallyAppliedVersion else {
          throw InputiaVoiceServiceError.policy
        }
      }
      let request = InputiaVoiceRequest(request_id: requestID, session_id: sessionID,
        server_instance: server.instance_id, client_instance: Self.processInstance,
        policy_epoch: locallyAppliedVersion?.policy_epoch ?? server.policy_epoch, command: command)
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
  static func checkPolicy(_ barrier: InputiaVoicePolicyBarrier, minimumEpoch: UInt64,
                          state: InputiaSharedStateBarrierApplying, sent: () -> Void) throws {
    try applyAndAcknowledge(barrier, minimumEpoch: minimumEpoch, state: state) { _ in sent() }
  }
  #endif
}
#endif
