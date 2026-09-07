#if INPUTIA_PAIRED_BUILD
import Foundation

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

  #if INPUTIA_CONNECTION_SELF_CHECK
  static func checkPolicy(_ barrier: InputiaVoicePolicyBarrier, minimumEpoch: UInt64,
                          state: InputiaSharedStateBarrierApplying, sent: () -> Void) throws {
    try applyAndAcknowledge(barrier, minimumEpoch: minimumEpoch, state: state) { _ in sent() }
  }
  #endif
}
#endif
