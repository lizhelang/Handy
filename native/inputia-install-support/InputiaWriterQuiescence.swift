import Darwin
import Foundation
import Security

// 只在存活租约内暂停通过固定角色和动态 audit-token 验证的旧写者，不终止进程。
// TIS、旧路径隔离和服务独占文件租约不在此证明内。
struct WriterQuiescenceRequest: Codable, Equatable {
  let schema_version: UInt32
  let action: String
  let subject: InstallSubject
  let epoch: String
  let old_release_id: String
  let roles: [InstallCodeRequest]
}
struct WriterProcessIdentity: Codable, Equatable {
  let pid: Int32
  let uid: UInt32
  let start_seconds: UInt64
  let start_microseconds: UInt64
  let pid_version: UInt32
  let role: String
  let bundle_id: String
  let release_id: String
  let executable_path: String
  let cdhash: String
}
struct WriterQuiescenceEvidence: Codable {
  let request: WriterQuiescenceRequest
  let suspended: [WriterProcessIdentity]
  let enumerated_roles: [String]
  let user_id: UInt32
  let rescanned_suspended: Bool
}
private struct WriterQuiescenceReply: Codable {
  let ok: Bool
  let evidence: WriterQuiescenceEvidence?
  let code: String?
  let os_status: Int32?
}
struct KernelWriterIdentity {
  let pid: Int32
  let uid: UInt32
  let parent: UInt32
  let startSeconds: UInt64
  let startMicroseconds: UInt64
  let token: audit_token_t
  let path: String
  var pidVersion: UInt32 { withUnsafeBytes(of: token) { $0.bindMemory(to: UInt32.self)[7] } }
}
func bsdInfo(_ pid: Int32) throws -> proc_bsdinfo {
  var info = proc_bsdinfo()
  let count = proc_pidinfo(pid, PROC_PIDTBSDINFO, 0, &info, Int32(MemoryLayout<proc_bsdinfo>.size))
  guard count == MemoryLayout<proc_bsdinfo>.size else {
    throw InstallCodeError.rejected(errno == ESRCH ? "process_gone" : "process_unreadable", errno)
  }
  return info
}
func captureKernelWriter(_ pid: Int32) throws -> KernelWriterIdentity {
  guard pid > 1, pid != getpid() else { throw InstallCodeError.rejected("unsafe_process") }
  let before = try bsdInfo(pid)
  guard before.pbi_uid == geteuid(), before.pbi_ruid == getuid() else {
    throw InstallCodeError.rejected("process_owner_mismatch")
  }
  var port = mach_port_t(MACH_PORT_NULL)
  let portStatus = task_name_for_pid(mach_task_self_, pid, &port)
  guard portStatus == KERN_SUCCESS, port != MACH_PORT_NULL else {
    throw InstallCodeError.rejected("audit_token_unavailable", portStatus)
  }
  defer { mach_port_deallocate(mach_task_self_, port) }
  var token = audit_token_t()
  var count = mach_msg_type_number_t(
    MemoryLayout<audit_token_t>.size / MemoryLayout<integer_t>.size)
  let tokenStatus = withUnsafeMutablePointer(to: &token) { pointer in
    pointer.withMemoryRebound(to: integer_t.self, capacity: Int(count)) {
      task_info(port, task_flavor_t(TASK_AUDIT_TOKEN), $0, &count)
    }
  }
  guard tokenStatus == KERN_SUCCESS,
    Int(count) * MemoryLayout<integer_t>.size == MemoryLayout<audit_token_t>.size
  else { throw InstallCodeError.rejected("audit_token_unavailable", tokenStatus) }
  let words = withUnsafeBytes(of: token) { Array($0.bindMemory(to: UInt32.self)) }
  guard words.count == 8, words[5] == UInt32(pid), words[1] == geteuid(), words[3] == getuid(),
    words[7] != 0
  else { throw InstallCodeError.rejected("audit_identity_mismatch") }
  var path = [CChar](repeating: 0, count: 4 * Int(MAXPATHLEN))
  guard proc_pidpath_audittoken(&token, &path, UInt32(path.count)) > 0 else {
    throw InstallCodeError.rejected("process_path_unavailable", errno)
  }
  let after = try bsdInfo(pid)
  guard before.pbi_pid == after.pbi_pid, before.pbi_uid == after.pbi_uid,
    before.pbi_start_tvsec == after.pbi_start_tvsec,
    before.pbi_start_tvusec == after.pbi_start_tvusec
  else { throw InstallCodeError.rejected("process_changed") }
  return KernelWriterIdentity(
    pid: pid, uid: before.pbi_uid, parent: before.pbi_ppid,
    startSeconds: before.pbi_start_tvsec, startMicroseconds: before.pbi_start_tvusec, token: token,
    path: String(cString: path))
}
private typealias AuditSignal =
  @convention(c) (UnsafeMutablePointer<audit_token_t>?, Int32) -> Int32
private func auditSignal() throws -> AuditSignal {
  // macOS 13 机器不一定提供此符号；能力不足必须失败，不能退回 kill(pid)。
  guard let pointer = dlsym(UnsafeMutableRawPointer(bitPattern: -2), "proc_signal_with_audittoken")
  else { throw InstallCodeError.rejected("audit_signal_unavailable") }
  return unsafeBitCast(pointer, to: AuditSignal.self)
}
func signalKernelWriter(
  _ identity: KernelWriterIdentity, signal: Int32, authorize: () -> Bool
) throws {
  let current = try captureKernelWriter(identity.pid)
  guard current.uid == identity.uid, current.startSeconds == identity.startSeconds,
    current.startMicroseconds == identity.startMicroseconds,
    current.pidVersion == identity.pidVersion,
    current.path == identity.path
  else { throw InstallCodeError.rejected("process_changed") }
  guard [SIGSTOP, SIGCONT].contains(signal), authorize() else {
    throw InstallCodeError.rejected("maintenance_authority_revoked")
  }
  var token = identity.token
  guard try auditSignal()(&token, signal) == 0 else {
    throw InstallCodeError.rejected("process_stop_failed", errno)
  }
}
func kernelWriterHasExited(_ identity: KernelWriterIdentity) throws -> Bool {
  do {
    let info = try bsdInfo(identity.pid)
    // PID 重新使用证明旧实例已结束，不代表新实例也被停止；上层还必须重新枚举。
    return info.pbi_start_tvsec != identity.startSeconds
      || info.pbi_start_tvusec != identity.startMicroseconds || info.pbi_status == SZOMB
  } catch InstallCodeError.rejected(let reason, let code) {
    if reason == "process_gone" || code == ESRCH { return true }
    throw InstallCodeError.rejected(reason, code)
  }
}
func validateWriterRequest(_ request: WriterQuiescenceRequest) throws {
  guard request.schema_version == 1, request.action == "suspend",
    UUID(uuidString: request.epoch)?.uuidString.lowercased() == request.epoch,
    request.epoch != "00000000-0000-0000-0000-000000000000",
    request.roles.map(\.role) == ["control", "ime", "settings"],
    Set(request.roles.map(\.bundle_id)).count == 3,
    Set(request.roles.map(\.exact_bundle_path)).count == 3
  else { throw InstallCodeError.rejected("writer_coverage_incomplete") }
  for role in request.roles {
    try validateInstallRequest(role)
    guard role.subject == request.subject, role.purpose == "previous_release",
      role.release_id == request.old_release_id
    else { throw InstallCodeError.rejected("writer_subject_mismatch") }
  }
}
struct VerifiedRunningWriter {
  let kernel: KernelWriterIdentity
  let evidence: WriterProcessIdentity
}
private func signingInfo(_ code: SecCode) throws -> [String: Any] {
  var staticCode: SecStaticCode?
  let staticStatus = SecCodeCopyStaticCode(code, [], &staticCode)
  guard staticStatus == errSecSuccess, let staticCode else {
    throw InstallCodeError.rejected("process_code_unavailable", staticStatus)
  }
  var raw: CFDictionary?
  let status = SecCodeCopySigningInformation(
    staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &raw)
  guard status == errSecSuccess, let info = raw as? [String: Any] else {
    throw InstallCodeError.rejected("process_code_unavailable", status)
  }
  return info
}
func verifyRunningWriter(_ pid: Int32, _ request: InstallCodeRequest) throws
  -> VerifiedRunningWriter
{
  let kernel = try captureKernelWriter(pid)
  var token = kernel.token
  let tokenData = withUnsafeBytes(of: &token) { Data($0) }
  var code: SecCode?
  var status = SecCodeCopyGuestWithAttributes(
    nil, [kSecGuestAttributeAudit: tokenData] as CFDictionary, [], &code)
  guard status == errSecSuccess, let code else {
    throw InstallCodeError.rejected("process_code_unavailable", status)
  }
  status = SecCodeCheckValidity(
    code, SecCSFlags(rawValue: kSecCSStrictValidate), try installRequirement(request))
  guard status == errSecSuccess else {
    throw InstallCodeError.rejected("process_code_rejected", status)
  }
  let info = try signingInfo(code)
  guard let executable = info[kSecCodeInfoMainExecutable as String] as? URL,
    let identifier = info[kSecCodeInfoIdentifier as String] as? String,
    let team = info[kSecCodeInfoTeamIdentifier as String] as? String,
    let hash = info[kSecCodeInfoUnique as String] as? Data,
    identifier == request.bundle_id, team == request.team_id,
    executable.path == kernel.path,
    executable.deletingLastPathComponent().path == request.exact_bundle_path + "/Contents/MacOS",
    request.cdhashes.contains(hash.map { String(format: "%02x", $0) }.joined())
  else { throw InstallCodeError.rejected("process_root_mismatch") }
  let after = try captureKernelWriter(pid)
  guard after.pidVersion == kernel.pidVersion, after.path == kernel.path,
    after.startSeconds == kernel.startSeconds, after.startMicroseconds == kernel.startMicroseconds
  else { throw InstallCodeError.rejected("process_changed") }
  return VerifiedRunningWriter(
    kernel: kernel,
    evidence: .init(
      pid: pid, uid: kernel.uid, start_seconds: kernel.startSeconds,
      start_microseconds: kernel.startMicroseconds, pid_version: kernel.pidVersion,
      role: request.role, bundle_id: identifier,
      release_id: request.release_id, executable_path: kernel.path,
      cdhash: hash.map { String(format: "%02x", $0) }.joined()))
}
struct WriterTreeNode {
  let pid: Int32
  let parent: UInt32
  let uid: UInt32
  let startSeconds: UInt64
  let startMicroseconds: UInt64
}
func processTreeSnapshot() throws -> [WriterTreeNode] {
  let capacity = proc_listallpids(nil, 0)
  guard capacity > 0, capacity < 100_000 else {
    throw InstallCodeError.rejected("process_inventory_unavailable")
  }
  var pids = [Int32](repeating: 0, count: Int(capacity) + 256)
  let count = pids.withUnsafeMutableBytes { proc_listallpids($0.baseAddress, Int32($0.count)) }
  guard count >= 0, count < pids.count else {
    throw InstallCodeError.rejected("process_inventory_changed")
  }
  var result: [WriterTreeNode] = []
  for pid in pids.prefix(Int(count)) where pid > 1 {
    do {
      let info = try bsdInfo(pid)
      if info.pbi_uid == geteuid(), info.pbi_status != SZOMB {
        result.append(
          .init(
            pid: pid, parent: info.pbi_ppid, uid: info.pbi_uid, startSeconds: info.pbi_start_tvsec,
            startMicroseconds: info.pbi_start_tvusec))
      }
    } catch InstallCodeError.rejected(let reason, _) where reason == "process_gone" { continue }
  }
  return result
}
func assertVerifiedWriterDescendants(_ roots: [KernelWriterIdentity], inventory: [WriterTreeNode])
  throws
{
  var authorized = Set<Int32>()
  for root in roots {
    guard let node = inventory.first(where: { $0.pid == root.pid }), node.uid == root.uid,
      node.startSeconds == root.startSeconds, node.startMicroseconds == root.startMicroseconds
    else { throw InstallCodeError.rejected("process_changed") }
    authorized.insert(root.pid)
  }
  // 全部受管角色作为根；任何非授权子孙（包括同uid外部helper）都阻止停写证明。
  for node in inventory
  where authorized.contains(Int32(bitPattern: node.parent)) && !authorized.contains(node.pid) {
    throw InstallCodeError.rejected("writer_descendant_unverified")
  }
}
func waitStopped(_ identity: KernelWriterIdentity) throws {
  let deadline = ProcessInfo.processInfo.systemUptime + 0.5
  while ProcessInfo.processInfo.systemUptime < deadline {
    let state = try bsdInfo(identity.pid)
    guard state.pbi_start_tvsec == identity.startSeconds,
      state.pbi_start_tvusec == identity.startMicroseconds
    else { throw InstallCodeError.rejected("process_changed") }
    if state.pbi_status == SSTOP { return }
    usleep(10_000)
  }
  throw InstallCodeError.rejected("writer_suspend_timeout")
}
func scanWriters(_ request: WriterQuiescenceRequest) throws -> [VerifiedRunningWriter] {
  let capacity = proc_listallpids(nil, 0)
  guard capacity > 0, capacity < 100_000 else {
    throw InstallCodeError.rejected("process_inventory_unavailable")
  }
  var pids = [Int32](repeating: 0, count: Int(capacity) + 256)
  let count = pids.withUnsafeMutableBytes { proc_listallpids($0.baseAddress, Int32($0.count)) }
  guard count >= 0, count < pids.count else {
    throw InstallCodeError.rejected("process_inventory_changed")
  }
  var writers: [VerifiedRunningWriter] = []
  for pid in pids.prefix(Int(count)) where pid > 1 {
    let owner: proc_bsdinfo
    do { owner = try bsdInfo(pid) } catch InstallCodeError.rejected(let reason, _)
      where reason == "process_gone"
    { continue }
    guard owner.pbi_uid == geteuid() else { continue }
    var path = [CChar](repeating: 0, count: 4 * Int(MAXPATHLEN))
    let pathCount = proc_pidpath(pid, &path, UInt32(path.count))
    if pathCount <= 0 {
      if (try? bsdInfo(pid)) == nil { continue }
      throw InstallCodeError.rejected("process_inventory_unreadable")
    }
    let actualPath = String(cString: path)
    // PID-only Security 仅做发现，不作为信任或停止授权；命中后重新绑定内核 audit token。
    var discovery: SecCode?
    let status = SecCodeCopyGuestWithAttributes(
      nil, [kSecGuestAttributePid: pid] as CFDictionary, [], &discovery)
    guard status == errSecSuccess, let discovery else {
      if (try? bsdInfo(pid)) == nil { continue }
      throw InstallCodeError.rejected("process_code_inventory_unreadable", status)
    }
    // 发现阶段不能吞元数据错误，否则受管根之外的旧套副本可能被当作不存在。
    let identifier = try signingInfo(discovery)[kSecCodeInfoIdentifier as String] as? String
    let root = request.roles.first { actualPath.hasPrefix($0.exact_bundle_path + "/") }
    let role = request.roles.first { $0.bundle_id == identifier }
    if let expected = role ?? root {
      guard writers.count < 64 else { throw InstallCodeError.rejected("writer_budget") }
      writers.append(try verifyRunningWriter(pid, expected))
    }
  }
  return writers.sorted { $0.evidence.pid < $1.evidence.pid }
}
// 此对象不能串成持久停写证明：进程崩溃不会执行 deinit，生产接线还需独立 guardian。
final class KernelWriterSuspension {
  private let observed: [KernelWriterIdentity]
  private var ownedStops: [KernelWriterIdentity] = []
  private var resumed = false
  private let recoveryFailure: () -> Void
  init(observed: [KernelWriterIdentity], recoveryFailure: @escaping () -> Void = {}) {
    self.observed = observed
    self.recoveryFailure = recoveryFailure
  }

  func suspend(authorize: () -> Bool, validateTree: () throws -> Void) throws {
    do {
      try validateTree()
      for identity in observed {
        // 已经 SSTOP 的进程不是本租约暂停的；恢复时绝不能擅自发 CONT。
        if try kernelWriterIsStopped(identity) { continue }
        try signalKernelWriter(identity, signal: SIGSTOP, authorize: authorize)
        ownedStops.append(identity)
        try waitStopped(identity)
      }
      try validateTree()
      try assertSuspended()
      guard authorize() else { throw InstallCodeError.rejected("maintenance_authority_revoked") }
    } catch {
      do { try resume() } catch {
        throw InstallCodeError.rejected("writer_resume_failed")
      }
      throw error
    }
  }
  func assertSuspended() throws {
    guard !resumed else { throw InstallCodeError.rejected("suspension_released") }
    for identity in observed {
      guard try kernelWriterIsStopped(identity) else {
        throw InstallCodeError.rejected("writer_not_suspended")
      }
    }
  }
  func resume() throws {
    var remaining: [KernelWriterIdentity] = []
    for identity in ownedStops {
      do {
        if try !kernelWriterHasExited(identity) {
          // 恢复只使用保存的 audit 实例；不需要已被撤销的维护授权，也不按PID退化。
          try signalKernelWriter(identity, signal: SIGCONT, authorize: { true })
          try waitResumed(identity)
        }
      } catch { remaining.append(identity) }
    }
    ownedStops = remaining
    guard remaining.isEmpty else { throw InstallCodeError.rejected("writer_resume_failed") }
    resumed = true
  }
  deinit {
    do { try resume() } catch {
      recoveryFailure()
      // Drop 只能尽力恢复，不能把失败谎报为成功；调用者须优先使用可重试的 resume。
      fputs("Inputia writer suspension cleanup failed; guardian recovery required\n", stderr)
    }
  }
}
func kernelWriterIsStopped(_ identity: KernelWriterIdentity) throws -> Bool {
  let current = try captureKernelWriter(identity.pid)
  guard current.pidVersion == identity.pidVersion,
    current.startSeconds == identity.startSeconds,
    current.startMicroseconds == identity.startMicroseconds,
    current.path == identity.path
  else { throw InstallCodeError.rejected("process_changed") }
  let state = try bsdInfo(identity.pid)
  guard state.pbi_start_tvsec == identity.startSeconds,
    state.pbi_start_tvusec == identity.startMicroseconds
  else { throw InstallCodeError.rejected("process_changed") }
  return state.pbi_status == SSTOP
}
private func waitResumed(_ identity: KernelWriterIdentity) throws {
  let deadline = ProcessInfo.processInfo.systemUptime + 0.5
  while ProcessInfo.processInfo.systemUptime < deadline {
    if try kernelWriterHasExited(identity) { return }
    if try !kernelWriterIsStopped(identity) { return }
    usleep(10_000)
  }
  throw InstallCodeError.rejected("writer_resume_failed")
}
// 同进程只能持有一个生产租约，避免一个拥有者的Drop解除另一拥有者的暂停。
final class WriterSuspensionSlot {
  private static let lock = NSLock()
  private static var busy = false
  private static var repairRequired = false
  private init() {}
  static func acquire() throws -> WriterSuspensionSlot {
    lock.lock()
    defer { lock.unlock() }
    guard !busy, !repairRequired else { throw InstallCodeError.rejected("suspension_already_held") }
    busy = true
    return WriterSuspensionSlot()
  }
  func poison() {
    Self.lock.lock()
    Self.repairRequired = true
    Self.lock.unlock()
  }
  deinit {
    Self.lock.lock()
    Self.busy = false
    Self.lock.unlock()
  }
}
private final class NativeWriterSuspension {
  let request: WriterQuiescenceRequest
  let writers: [VerifiedRunningWriter]
  let kernel: KernelWriterSuspension
  private let slot: WriterSuspensionSlot
  init(request: WriterQuiescenceRequest, authorize: () -> Bool) throws {
    try validateWriterRequest(request)
    for role in request.roles { _ = try verifyInstallCode(role) }
    _ = try auditSignal()  // 缺恢复能力时不得先暂停。
    let acquiredSlot = try WriterSuspensionSlot.acquire()
    slot = acquiredSlot
    self.request = request
    writers = try scanWriters(request)
    kernel = KernelWriterSuspension(
      observed: writers.map(\.kernel), recoveryFailure: { acquiredSlot.poison() })
    try kernel.suspend(authorize: authorize) {
      try assertVerifiedWriterDescendants(
        self.writers.map(\.kernel), inventory: processTreeSnapshot())
    }
    try assertSuspended(authorize: authorize)
  }
  func assertSuspended(authorize: () -> Bool) throws {
    guard authorize() else { throw InstallCodeError.rejected("maintenance_authority_revoked") }
    for role in request.roles { _ = try verifyInstallCode(role) }
    try kernel.assertSuspended()
    let now = try scanWriters(request)
    guard now.map(\.evidence) == writers.map(\.evidence) else {
      throw InstallCodeError.rejected("writer_set_changed")
    }
    try assertVerifiedWriterDescendants(writers.map(\.kernel), inventory: processTreeSnapshot())
    try kernel.assertSuspended()
    guard authorize() else { throw InstallCodeError.rejected("maintenance_authority_revoked") }
  }
  var evidence: WriterQuiescenceEvidence {
    .init(
      request: request, suspended: writers.map(\.evidence),
      enumerated_roles: request.roles.map(\.role), user_id: geteuid(), rescanned_suspended: true)
  }
}
public typealias WriterEffectCheck = @convention(c) (UnsafeMutableRawPointer?) -> Int32
private func suspensionReply(_ body: () throws -> WriterQuiescenceEvidence) -> UnsafeMutablePointer<
  CChar
>? {
  let reply: WriterQuiescenceReply
  do {
    reply = .init(ok: true, evidence: try body(), code: nil, os_status: nil)
  } catch InstallCodeError.rejected(let reason, let status) {
    reply = .init(ok: false, evidence: nil, code: reason, os_status: status)
  } catch { reply = .init(ok: false, evidence: nil, code: "invalid_request", os_status: 0) }
  guard let data = try? installCanonical(reply), let text = String(data: data, encoding: .utf8)
  else { return nil }
  return strdup(text)
}
@_cdecl("iuis_writer_suspend")
public func iuisWriterSuspend(
  _ bytes: UnsafePointer<UInt8>?, _ length: UInt, _ effectCheck: WriterEffectCheck?,
  _ context: UnsafeMutableRawPointer?, _ handle: UnsafeMutablePointer<UnsafeMutableRawPointer?>?
) -> UnsafeMutablePointer<CChar>? {
  handle?.pointee = nil
  var lease: NativeWriterSuspension?
  let reply = suspensionReply {
    guard let bytes, let effectCheck, let context, handle != nil,
      length > 0, length <= 131_072
    else { throw InstallCodeError.rejected("invalid_request") }
    let raw = Data(bytes: bytes, count: Int(length))
    let request = try JSONDecoder().decode(WriterQuiescenceRequest.self, from: raw)
    guard try installCanonical(request) == raw else {
      throw InstallCodeError.rejected("noncanonical_request")
    }
    let acquired = try NativeWriterSuspension(
      request: request, authorize: { effectCheck(context) == 0 })
    lease = acquired
    return acquired.evidence
  }
  if reply != nil, let lease { handle?.pointee = Unmanaged.passRetained(lease).toOpaque() }
  return reply
}
@_cdecl("iuis_writer_assert_suspended")
public func iuisWriterAssertSuspended(
  _ handle: UnsafeMutableRawPointer?, _ effectCheck: WriterEffectCheck?,
  _ context: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<CChar>? {
  suspensionReply {
    guard let handle, let effectCheck, let context else {
      throw InstallCodeError.rejected("invalid_request")
    }
    let lease = Unmanaged<NativeWriterSuspension>.fromOpaque(handle).takeUnretainedValue()
    try lease.assertSuspended(authorize: { effectCheck(context) == 0 })
    return lease.evidence
  }
}
private struct WriterResumeReply: Codable {
  let ok: Bool
  let code: String?
  let os_status: Int32?
}
@_cdecl("iuis_writer_resume")
public func iuisWriterResume(_ handle: UnsafeMutableRawPointer?) -> UnsafeMutablePointer<CChar>? {
  let reply: WriterResumeReply
  do {
    guard let handle else { throw InstallCodeError.rejected("invalid_request") }
    try Unmanaged<NativeWriterSuspension>.fromOpaque(handle).takeUnretainedValue().kernel.resume()
    reply = .init(ok: true, code: nil, os_status: nil)
  } catch InstallCodeError.rejected(let code, let status) {
    reply = .init(ok: false, code: code, os_status: status)
  } catch { reply = .init(ok: false, code: "writer_resume_failed", os_status: 0) }
  guard let data = try? installCanonical(reply), let text = String(data: data, encoding: .utf8)
  else { return nil }
  return strdup(text)
}
@_cdecl("iuis_writer_suspension_free")
public func iuisWriterSuspensionFree(_ handle: UnsafeMutableRawPointer?) {
  if let handle { Unmanaged<NativeWriterSuspension>.fromOpaque(handle).release() }
}
