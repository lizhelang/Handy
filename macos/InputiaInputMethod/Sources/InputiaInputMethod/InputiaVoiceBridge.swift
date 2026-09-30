#if INPUTIA_PAIRED_BUILD
import Foundation

/// 一条串行认证连接；所有socket工作在后台，所有CAPI/宿主状态回调回主线程。
final class InputiaVoiceBridge {
  static let shared = InputiaVoiceBridge()
  private let queue = DispatchQueue(label: "Inputia.managed-memory")
  private let work = InputiaMemoryPendingWork()
  private var connection: InputiaVoiceServiceConnection?
  private func connected() throws -> InputiaVoiceServiceConnection {
    if let connection { return connection }
    let value = try InputiaVoiceInputLauncher.openAuthenticatedConnection()
    guard value.server.capabilities.contains("memory_domain_v1") else { value.close(); throw InputiaMemoryError.unavailable }
    connection = value
    return value
  }
  private static func onMain(_ body: @escaping () -> Void) {
    if Thread.isMainThread { body() } else { DispatchQueue.main.async(execute: body) }
  }
  private func failed() {
    connection?.closeTypedCaptureConnection(); connection = nil
    DispatchQueue.main.async { InputiaMemoryBarrier.invalidate?() }
  }
  private func policy(_ connection: InputiaVoiceServiceConnection) throws -> InputiaMemoryPolicy {
    guard let version = connection.locallyAppliedVersion else { throw InputiaMemoryError.unavailable }
    return .init(server_instance: connection.server.instance_id, profile_id: connection.server.profile_id, policy_epoch: version.policy_epoch)
  }
  func prepare(_ completion: @escaping (Result<InputiaMemoryPolicy, Error>) -> Void) {
    queue.async {
      do { let policy = try self.policy(self.connected()); DispatchQueue.main.async { completion(.success(policy)) } }
      catch { self.failed(); DispatchQueue.main.async { completion(.failure(error)) } }
    }
  }
  func query(ticket: InputiaMemoryTicket, target: InputiaMemoryTarget, started: TimeInterval,
             completion: @escaping (Result<InputiaMemorySnapshot, Error>) -> Void) {
    let request = InputiaMemoryRequest(request_id: UUID().uuidString,
      client_instance: InputiaVoiceServiceConnection.processInstance, server_instance: ticket.policy.server_instance,
      policy_epoch: ticket.policy.policy_epoch, memory_domain: .init(kind: "query", query_id: UUID().uuidString,
        query_generation: ticket.ticket, target: target, composing: ticket.composing, query: ticket.query))
    work.enqueue(on: queue, cancelled: { Self.onMain { completion(.failure(InputiaMemoryError.retired)) } }) { token in
      do {
        let connection = try self.connected()
        guard token.valid, try self.policy(connection) == ticket.policy,
          ProcessInfo.processInfo.systemUptime - started < 2 else { throw InputiaMemoryError.retired }
        let reply = try connection.memoryDomain(request)
        guard token.valid, reply.code == nil, reply.result?.kind == "snapshot", let snapshot = reply.result?.snapshot else { throw InputiaMemoryError.unavailable }
        try snapshot.validate(request: request, ticket: ticket, started: started, now: ProcessInfo.processInfo.systemUptime)
        self.work.enqueue(on: .main, cancelled: { completion(.failure(InputiaMemoryError.retired)) }) { delivery in
          do {
            guard token.valid, delivery.valid else { throw InputiaMemoryError.retired }
            try snapshot.validate(request: request, ticket: ticket, started: started, now: ProcessInfo.processInfo.systemUptime); completion(.success(snapshot)) }
          catch { completion(.failure(error)) }
        }
      } catch {
        if case InputiaMemoryError.retired = error {} else { self.failed() }
        DispatchQueue.main.async { completion(.failure(error)) }
      }
    }
  }
  func management(_ command: InputiaMemoryCommand, expectedEpoch: UInt64? = nil,
                  completion: @escaping (Result<InputiaMemoryReply, Error>) -> Void) {
    work.enqueue(on: queue, cancelled: { Self.onMain { completion(.failure(InputiaMemoryError.retired)) } }) { token in
      do {
        let connection = try self.connected(), policy = try self.policy(connection)
        guard token.valid else { throw InputiaMemoryError.retired }
        let epoch = expectedEpoch ?? policy.policy_epoch
        if command.kind != "outcome" && epoch != policy.policy_epoch { throw InputiaMemoryError.retired }
        let request = InputiaMemoryRequest(request_id: UUID().uuidString,
          client_instance: InputiaVoiceServiceConnection.processInstance, server_instance: policy.server_instance,
          policy_epoch: epoch, memory_domain: command)
        let reply = try connection.memoryDomain(request)
        guard token.valid else { throw InputiaMemoryError.retired }
        self.work.enqueue(on: .main, cancelled: { completion(.failure(InputiaMemoryError.retired)) }) { delivery in
          if token.valid && delivery.valid { completion(.success(reply)) }
          else { completion(.failure(InputiaMemoryError.retired)) }
        }
      } catch {
        if case InputiaMemoryError.retired = error {} else { self.failed() }
        DispatchQueue.main.async { completion(.failure(error)) }
      }
    }
  }
  func retirePending() { work.cancelAll() }
  func invalidate() { retirePending(); queue.async { self.failed() } }
}

extension InputiaMemoryTarget {
  init(_ value: InputiaVoiceTarget) {
    self.init(target_id: value.target_id, host_instance: value.host_instance, controller_id: value.controller_id,
      activation_generation: value.activation_generation, field_id: value.field_id,
      selection_generation: value.selection_generation, composition_generation: value.composition_generation, source_app: value.source_app)
  }
}
#endif
