import Darwin
import Foundation
import Security

// 原生 plan 只由本机扫描的已验三角色产生。协议只能引用其index，不接受JSON PID发信号。
struct GuardianNativeConfiguration: Codable {
  let writers: WriterQuiescenceRequest
  let updater: InstallCodeRequest
}
struct GuardianNativeEntry: Codable, Equatable {
  let index: UInt32
  let identity: WriterProcessIdentity
  let initialState: String
  enum CodingKeys: String, CodingKey {
    case index, identity
    case initialState = "initial_state"
  }
}
struct GuardianNativeInventory: Codable {
  let entries: [GuardianNativeEntry]
  let executablePath: String
  enum CodingKeys: String, CodingKey {
    case entries
    case executablePath = "executable_path"
  }
}
private struct GuardianNativeReply<T: Codable>: Codable {
  let ok: Bool
  let value: T?
  let code: String?
  let osStatus: Int32?
  enum CodingKeys: String, CodingKey {
    case ok, value, code
    case osStatus = "os_status"
  }
}
private func guardianReply<T: Codable>(_ action: () throws -> T) -> UnsafeMutablePointer<CChar>? {
  let reply: GuardianNativeReply<T>
  do {
    reply = .init(ok: true, value: try action(), code: nil, osStatus: nil)
  } catch InstallCodeError.rejected(let code, let status) {
    reply = .init(ok: false, value: nil, code: code, osStatus: status)
  } catch { reply = .init(ok: false, value: nil, code: "guardian_native_failed", osStatus: 0) }
  guard let bytes = try? installCanonical(reply), let text = String(data: bytes, encoding: .utf8)
  else { return nil }
  return strdup(text)
}
private func guardianDecode<T: Codable>(
  _ type: T.Type, _ bytes: UnsafePointer<UInt8>?, _ count: UInt
) throws -> T {
  guard let bytes, count > 0, count <= 131_072 else {
    throw InstallCodeError.rejected("invalid_request")
  }
  let raw = Data(bytes: bytes, count: Int(count))
  let value = try JSONDecoder().decode(type, from: raw)
  guard try installCanonical(value) == raw else {
    throw InstallCodeError.rejected("noncanonical_request")
  }
  return value
}
private func guardianSelfExecutable(_ expected: InstallCodeRequest) throws -> String {
  try validateInstallRequest(expected)
  guard expected.role == "updater" else {
    throw InstallCodeError.rejected("guardian_updater_required")
  }
  _ = try verifyInstallCode(expected)
  var code: SecCode?
  guard SecCodeCopySelf([], &code) == errSecSuccess, let code,
    SecCodeCheckValidity(
      code, SecCSFlags(rawValue: kSecCSStrictValidate), try installRequirement(expected))
      == errSecSuccess
  else { throw InstallCodeError.rejected("guardian_self_identity") }
  var staticCode: SecStaticCode?
  guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode else {
    throw InstallCodeError.rejected("guardian_self_identity")
  }
  var raw: CFDictionary?
  guard
    SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &raw)
      == errSecSuccess,
    let info = raw as? [String: Any],
    let executable = info[kSecCodeInfoMainExecutable as String] as? URL,
    executable.deletingLastPathComponent().path == expected.exact_bundle_path + "/Contents/MacOS"
  else { throw InstallCodeError.rejected("guardian_self_identity") }
  return executable.path
}
final class GuardianNativePlan {
  let writers: [VerifiedRunningWriter]
  let entries: [GuardianNativeEntry]
  let request: WriterQuiescenceRequest?
  let executable: String
  private let slot: WriterSuspensionSlot?
  private var effectsClosed = false
  private var issued: Set<UInt32> = []

  convenience init(configuration: GuardianNativeConfiguration) throws {
    let slot = try WriterSuspensionSlot.acquire()
    try validateWriterRequest(configuration.writers)
    guard configuration.updater.subject == configuration.writers.subject else {
      throw InstallCodeError.rejected("writer_subject_mismatch")
    }
    let executable = try guardianSelfExecutable(configuration.updater)
    for role in configuration.writers.roles { _ = try verifyInstallCode(role) }
    let writers = try scanWriters(configuration.writers)
    try assertVerifiedWriterDescendants(writers.map(\.kernel), inventory: processTreeSnapshot())
    try self.init(
      writers: writers, request: configuration.writers, executable: executable, slot: slot)
  }
  // 内部初始化也供只链接到自检可执行文件的夹具使用；C ABI没有任意PID构造。
  init(
    writers: [VerifiedRunningWriter], request: WriterQuiescenceRequest?, executable: String,
    slot: WriterSuspensionSlot? = nil
  )
    throws
  {
    guard writers.count <= 64 else { throw InstallCodeError.rejected("writer_budget") }
    self.slot = slot
    self.writers = writers
    self.request = request
    self.executable = executable
    entries = try writers.enumerated().map { index, writer in
      .init(
        index: UInt32(index), identity: writer.evidence,
        initialState: try kernelWriterIsStopped(writer.kernel) ? "observed_stopped" : "running")
    }
  }
  private func writer(_ index: UInt32) throws -> VerifiedRunningWriter {
    guard Int(index) < writers.count else {
      throw InstallCodeError.rejected("guardian_unknown_index")
    }
    return writers[Int(index)]
  }
  func stop(_ index: UInt32, authorize: () -> Bool) throws {
    guard !effectsClosed, entries.indices.contains(Int(index)),
      entries[Int(index)].initialState == "running",
      !issued.contains(index)
    else { throw InstallCodeError.rejected("guardian_effect_closed") }
    let known = try writer(index)
    if let request, let expected = request.roles.first(where: { $0.role == known.evidence.role }) {
      guard try verifyRunningWriter(known.kernel.pid, expected).evidence == known.evidence else {
        throw InstallCodeError.rejected("process_changed")
      }
    }
    // syscall之前就记录已尝试；结果未知不得再发STOP，只能进入恢复。
    issued.insert(index)
    try signalKernelWriter(known.kernel, signal: SIGSTOP, authorize: authorize)
    try waitStopped(known.kernel)
  }
  func closeEffects() { effectsClosed = true }
  func state(_ index: UInt32) throws -> String {
    let known = try writer(index)
    if try kernelWriterHasExited(known.kernel) { return "original_exited" }
    let now = try captureKernelWriter(known.kernel.pid)
    if now.pidVersion != known.kernel.pidVersion || now.startSeconds != known.kernel.startSeconds
      || now.startMicroseconds != known.kernel.startMicroseconds
    {
      return "original_exited"
    }
    guard now.path == known.kernel.path else { throw InstallCodeError.rejected("process_changed") }
    return try kernelWriterIsStopped(known.kernel) ? "stopped" : "running"
  }
  func resume(_ index: UInt32) throws -> String {
    guard effectsClosed, entries.indices.contains(Int(index)),
      entries[Int(index)].initialState == "running"
    else { throw InstallCodeError.rejected("guardian_effect_closed") }
    let prior = try state(index)
    if prior != "stopped" { return prior }
    try signalKernelWriter(writer(index).kernel, signal: SIGCONT, authorize: { true })
    let deadline = ProcessInfo.processInfo.systemUptime + 0.5
    while ProcessInfo.processInfo.systemUptime < deadline {
      let current = try state(index)
      if current != "stopped" { return current == "running" ? "resumed" : current }
      usleep(10_000)
    }
    throw InstallCodeError.rejected("writer_resume_failed")
  }
  func assertHolding() throws {
    guard !effectsClosed else { throw InstallCodeError.rejected("guardian_effect_closed") }
    for entry in entries {
      guard try state(entry.index) == "stopped" else {
        throw InstallCodeError.rejected("writer_not_suspended")
      }
    }
    if let request {
      guard try scanWriters(request).map(\.evidence) == writers.map(\.evidence) else {
        throw InstallCodeError.rejected("writer_set_changed")
      }
      try assertVerifiedWriterDescendants(writers.map(\.kernel), inventory: processTreeSnapshot())
    }
  }
}
private final class GuardianNativePeer {
  let writer: VerifiedRunningWriter
  init(pid: Int32, expected: InstallCodeRequest) throws {
    guard expected.role == "updater" else {
      throw InstallCodeError.rejected("guardian_updater_required")
    }
    _ = try guardianSelfExecutable(expected)
    writer = try verifyRunningWriter(pid, expected)
  }
  func state() throws -> String {
    if try kernelWriterHasExited(writer.kernel) { return "exited" }
    let now = try captureKernelWriter(writer.kernel.pid)
    if now.pidVersion != writer.kernel.pidVersion { return "exec" }
    guard now.path == writer.kernel.path, now.startSeconds == writer.kernel.startSeconds,
      now.startMicroseconds == writer.kernel.startMicroseconds
    else { throw InstallCodeError.rejected("process_changed") }
    return "alive"
  }
}
@_cdecl("iuis_guardian_prepare")
public func iuisGuardianPrepare(
  _ bytes: UnsafePointer<UInt8>?, _ count: UInt,
  _ output: UnsafeMutablePointer<UnsafeMutableRawPointer?>?
) -> UnsafeMutablePointer<CChar>? {
  output?.pointee = nil
  var plan: GuardianNativePlan?
  let result = guardianReply { () throws -> GuardianNativeInventory in
    guard output != nil else { throw InstallCodeError.rejected("invalid_request") }
    let value = try GuardianNativePlan(
      configuration: guardianDecode(GuardianNativeConfiguration.self, bytes, count))
    plan = value
    return .init(entries: value.entries, executablePath: value.executable)
  }
  if result != nil, let plan { output?.pointee = Unmanaged.passRetained(plan).toOpaque() }
  return result
}
@_cdecl("iuis_guardian_plan_action")
public func iuisGuardianPlanAction(
  _ handle: UnsafeMutableRawPointer?, _ action: UInt32, _ index: UInt32,
  _ check: WriterEffectCheck?, _ context: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<CChar>? {
  guardianReply {
    guard let handle else { throw InstallCodeError.rejected("invalid_request") }
    let plan = Unmanaged<GuardianNativePlan>.fromOpaque(handle).takeUnretainedValue()
    switch action {
    case 1:
      guard let check, let context else { throw InstallCodeError.rejected("invalid_request") }
      try plan.stop(index, authorize: { check(context) == 0 })
      return "stopped"
    case 2:
      plan.closeEffects()
      return "recovering"
    case 3: return try plan.resume(index)
    case 4: return try plan.state(index)
    case 5:
      try plan.assertHolding()
      return "holding"
    default: throw InstallCodeError.rejected("invalid_request")
    }
  }
}
@_cdecl("iuis_guardian_plan_free")
public func iuisGuardianPlanFree(_ handle: UnsafeMutableRawPointer?) {
  if let handle { Unmanaged<GuardianNativePlan>.fromOpaque(handle).release() }
}
@_cdecl("iuis_guardian_peer_open")
public func iuisGuardianPeerOpen(
  _ pid: Int32, _ bytes: UnsafePointer<UInt8>?, _ count: UInt,
  _ output: UnsafeMutablePointer<UnsafeMutableRawPointer?>?
) -> UnsafeMutablePointer<CChar>? {
  output?.pointee = nil
  var peer: GuardianNativePeer?
  let result = guardianReply { () throws -> WriterProcessIdentity in
    guard output != nil else { throw InstallCodeError.rejected("invalid_request") }
    let value = try GuardianNativePeer(
      pid: pid, expected: guardianDecode(InstallCodeRequest.self, bytes, count))
    peer = value
    return value.writer.evidence
  }
  if result != nil, let peer { output?.pointee = Unmanaged.passRetained(peer).toOpaque() }
  return result
}
@_cdecl("iuis_guardian_peer_state")
public func iuisGuardianPeerState(_ handle: UnsafeMutableRawPointer?) -> UnsafeMutablePointer<
  CChar
>? {
  guardianReply {
    guard let handle else { throw InstallCodeError.rejected("invalid_request") }
    return try Unmanaged<GuardianNativePeer>.fromOpaque(handle).takeUnretainedValue().state()
  }
}
@_cdecl("iuis_guardian_peer_free")
public func iuisGuardianPeerFree(_ handle: UnsafeMutableRawPointer?) {
  if let handle { Unmanaged<GuardianNativePeer>.fromOpaque(handle).release() }
}
