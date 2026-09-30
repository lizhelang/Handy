import Carbon
import CryptoKit
import Darwin
import Foundation
import Security

// TIS没有原子CAS或选择所有者计数。此状态机只提供当次读回观察，不构成绝对恢复证明。
struct InputSourceDescriptor: Equatable {
  let id: String
  let token: String
  let target: Bool
  let verifiedApple: Bool
  let enabled: Bool
  let selectable: Bool
}
protocol InputSourceBackend: AnyObject {
  func current() throws -> InputSourceDescriptor
  func fallback() throws -> InputSourceDescriptor
  func original(_ id: String) throws -> InputSourceDescriptor
  func select(
    _ source: InputSourceDescriptor, expecting: InputSourceDescriptor, generation: UInt64,
    authorize: () -> Bool) throws
  func generation() -> UInt64
  func now() -> UInt64
}
struct InputSourceObservation: Codable {
  var state: String
  var originalSourceID: String
  var observedSourceID: String?
  var fallbackSourceID: String?
  var selectionAttempted = false
  var restorationAttempted = false
  var reason: String?
  // 不存在TIS级比较交换；同ID ABA、通知延迟及最终检查后的竞争无法排除。
  let ownershipExact = false
  enum CodingKeys: String, CodingKey {
    case state, reason
    case originalSourceID = "original_source_id"
    case observedSourceID = "observed_source_id"
    case fallbackSourceID = "fallback_source_id"
    case selectionAttempted = "selection_attempted"
    case restorationAttempted = "restoration_attempted"
    case ownershipExact = "ownership_exact"
  }
}
final class InputSourceSlot {
  private static let lock = NSLock()
  private static var busy = false
  private init() {}
  static func acquire() throws -> InputSourceSlot {
    lock.lock()
    defer { lock.unlock() }
    guard !busy else { throw InstallCodeError.rejected("input_source_lease_busy") }
    busy = true
    return InputSourceSlot()
  }
  deinit {
    Self.lock.lock()
    Self.busy = false
    Self.lock.unlock()
  }
}
final class InputSourceMachine {
  let backend: InputSourceBackend
  let prior: InputSourceDescriptor
  let deadline: UInt64
  private let slot: InputSourceSlot
  private var detached: InputSourceDescriptor?
  private var baseline: UInt64
  private(set) var observation: InputSourceObservation

  init(backend: InputSourceBackend, deadline: UInt64, slot: InputSourceSlot) throws {
    self.backend = backend
    self.deadline = deadline
    self.slot = slot
    let before = backend.generation()
    prior = try backend.current()
    guard before != UInt64.max, before == backend.generation(), backend.now() < deadline else {
      throw InstallCodeError.rejected("input_source_observation_changed")
    }
    baseline = before
    observation = .init(state: "prepared", originalSourceID: prior.id, observedSourceID: prior.id)
  }
  private func authorized(_ authorize: () -> Bool) -> Bool {
    authorize() && backend.generation() != UInt64.max && backend.now() < deadline
  }
  private func uncertain(_ reason: String) -> InputSourceObservation {
    observation.state = "uncertain"
    observation.reason = reason
    return observation
  }
  private func observe() throws -> InputSourceDescriptor {
    let value = try backend.current()
    observation.observedSourceID = value.id
    return value
  }
  func detach(authorize: () -> Bool) -> InputSourceObservation {
    if ["already_detached", "detached_observed", "preserved_user_selection"].contains(
      observation.state)
    {
      return assertDetached(authorize: authorize)
    }
    guard observation.state == "prepared" else { return observation }
    do {
      let current = try observe()
      guard current == prior, baseline == backend.generation(), authorized(authorize) else {
        return uncertain("input_source_observation_changed")
      }
      if !current.target {
        observation.state = "already_detached"
        return observation
      }
      let fallback = try backend.fallback()
      guard fallback.verifiedApple, !fallback.target, fallback.enabled, fallback.selectable else {
        return uncertain("no_verified_fallback")
      }
      // 慢校验后再读原选择和通知版本，最后再核维护与期限。
      guard try observe() == prior, baseline == backend.generation(), authorized(authorize) else {
        return uncertain("input_source_observation_changed")
      }
      detached = fallback
      observation.fallbackSourceID = fallback.id
      observation.selectionAttempted = true  // 在不可撤销调用前标未知，失败不自动重放。
      try backend.select(
        fallback, expecting: prior, generation: baseline, authorize: { authorized(authorize) })
      let observed = try observe()
      guard observed == fallback, authorized(authorize) else {
        return uncertain("detach_result_uncertain")
      }
      // 即使通知看似由自身选择引起，也不据此抹去并发选择/ABA的证据。
      guard baseline == backend.generation() else { return uncertain("selection_history_unknown") }
      observation.state = "detached_observed"
      return observation
    } catch InstallCodeError.rejected(let reason, _) { return uncertain(reason) } catch {
      return uncertain("input_source_backend_failed")
    }
  }
  func assertDetached(authorize: () -> Bool) -> InputSourceObservation {
    guard
      ["already_detached", "detached_observed", "preserved_user_selection"].contains(
        observation.state)
    else {
      return observation
    }
    do {
      let observed = try observe()
      guard !observed.target, authorized(authorize) else {
        return uncertain("detach_no_longer_observed")
      }
      if let detached, observed != detached {
        observation.state = "preserved_user_selection"
        observation.reason = "selection_changed"
      } else if detached != nil && baseline != backend.generation() {
        return uncertain("selection_history_unknown")
      }
      return observation
    } catch { return uncertain("input_source_backend_failed") }
  }
  func restore(authorize: () -> Bool) -> InputSourceObservation {
    if observation.state == "already_detached" { return assertDetached(authorize: authorize) }
    if observation.state == "prepared" { return observation }
    if observation.state == "restored_observed" {
      do {
        let current = try observe()
        guard authorized(authorize) else { return uncertain("restore_authority_changed") }
        if current.id != prior.id {
          observation.state = "preserved_user_selection"
          observation.reason = "selection_changed"
        } else if baseline != backend.generation() {
          return uncertain("selection_history_unknown")
        }
        return observation
      } catch { return uncertain("input_source_backend_failed") }
    }
    guard observation.state == "detached_observed", !observation.restorationAttempted,
      let detached
    else { return observation }
    do {
      let current = try observe()
      guard current == detached else {
        observation.state = "preserved_user_selection"
        observation.reason = "selection_changed"
        return observation
      }
      guard baseline == backend.generation() else { return uncertain("selection_history_unknown") }
      let original = try backend.original(prior.id)
      guard original.target, original.enabled, original.selectable else {
        return uncertain("original_source_unavailable")
      }
      guard try observe() == detached, baseline == backend.generation(), authorized(authorize)
      else {
        return uncertain("restore_authority_changed")
      }
      observation.restorationAttempted = true
      try backend.select(
        original, expecting: detached, generation: baseline, authorize: { authorized(authorize) })
      let observed = try observe()
      guard observed == original, authorized(authorize) else {
        return uncertain("restore_result_uncertain")
      }
      guard baseline == backend.generation() else { return uncertain("selection_history_unknown") }
      observation.state = "restored_observed"
      return observation
    } catch InstallCodeError.rejected(let reason, _) { return uncertain(reason) } catch {
      return uncertain("input_source_backend_failed")
    }
  }
  // 不在deinit中选择输入源；退出/崩溃恢复尚未与guardian联动。
}

private let appleLayoutRoot = "/System/Library/Keyboard Layouts/AppleKeyboardLayouts.bundle"
private func sourceString(_ source: TISInputSource, _ key: CFString) throws -> String? {
  guard let pointer = TISGetInputSourceProperty(source, key) else { return nil }
  let value = Unmanaged<CFTypeRef>.fromOpaque(pointer).takeUnretainedValue()
  guard CFGetTypeID(value) == CFStringGetTypeID(), let result = value as? String,
    !result.isEmpty, result.utf8.count <= 4096,
    !result.unicodeScalars.contains(where: { $0.value < 32 })
  else { throw InstallCodeError.rejected("input_source_invalid_property") }
  return result
}
private func sourceBool(_ source: TISInputSource, _ key: CFString) throws -> Bool {
  guard let pointer = TISGetInputSourceProperty(source, key) else { return false }
  let value = Unmanaged<CFTypeRef>.fromOpaque(pointer).takeUnretainedValue()
  guard CFGetTypeID(value) == CFBooleanGetTypeID() else {
    throw InstallCodeError.rejected("input_source_invalid_property")
  }
  return CFBooleanGetValue(unsafeBitCast(value, to: CFBoolean.self))
}
private func sourceURL(_ source: TISInputSource) throws -> URL? {
  guard let pointer = TISGetInputSourceProperty(source, kTISPropertyIconImageURL) else {
    return nil
  }
  let value = Unmanaged<CFTypeRef>.fromOpaque(pointer).takeUnretainedValue()
  guard CFGetTypeID(value) == CFURLGetTypeID(), let url = value as? URL, url.isFileURL else {
    throw InstallCodeError.rejected("input_source_invalid_property")
  }
  return url.standardizedFileURL
}
private func selectedGeneration(
  _ center: CFNotificationCenter?, _ observer: UnsafeMutableRawPointer?,
  _ name: CFNotificationName?, _ object: UnsafeRawPointer?, _ userInfo: CFDictionary?
) {
  guard let observer else { return }
  Unmanaged<SourceNotifications>.fromOpaque(observer).takeUnretainedValue().advance()
}
private final class SourceNotifications {
  private let lock = NSLock()
  private var count: UInt64 = 0
  init() {
    CFNotificationCenterAddObserver(
      CFNotificationCenterGetDistributedCenter(),
      Unmanaged.passUnretained(self).toOpaque(), selectedGeneration,
      kTISNotifySelectedKeyboardInputSourceChanged, nil, .deliverImmediately)
  }
  func advance() {
    lock.lock()
    if count < UInt64.max { count += 1 }
    lock.unlock()
  }
  func generation() -> UInt64 {
    lock.lock()
    defer { lock.unlock() }
    return count
  }
  deinit {
    CFNotificationCenterRemoveEveryObserver(
      CFNotificationCenterGetDistributedCenter(),
      Unmanaged.passUnretained(self).toOpaque())
  }
}
struct SourceContract {
  let expectation: InstallCodeRequest
  let icons: [String: URL]
  static func verified(_ request: InstallCodeRequest) throws -> SourceContract {
    try validateInstallRequest(request)
    guard request.role == "ime" else {
      throw InstallCodeError.rejected("input_source_ime_required")
    }
    var contract: SourceContract?
    _ = try verifyInstallCode(request) { securedPlist in
      contract = try fromSecuredPlist(securedPlist, request: request)
    }
    guard let contract else { throw InstallCodeError.rejected("secured_plist_unavailable") }
    return contract
  }
  // 生产唯一调用点消费同一已验代码的 kSecCodeInfoPList；测试可注入字典，不生成任何选择能力。
  // Security 自身加载签名元数据；这里只对真正使用的合同字段施加预算，不承诺限制其内部读取。
  static func fromSecuredPlist(_ plist: [String: Any], request: InstallCodeRequest) throws
    -> SourceContract
  {
    guard plist["CFBundleIdentifier"] as? String == request.bundle_id,
      plist["TISInputSourceID"] as? String == request.bundle_id,
      let component = plist["ComponentInputModeDict"] as? [String: Any],
      let modes = component["tsInputModeListKey"] as? [String: [String: Any]],
      !modes.isEmpty, modes.count <= 32
    else { throw InstallCodeError.rejected("input_source_contract_mismatch") }
    var icons: [String: URL] = [:]
    for (key, mode) in modes {
      guard key.hasPrefix(request.bundle_id + "."), key.utf8.count <= 256,
        !key.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains),
        mode["TISInputSourceID"] as? String == key,
        let icon = mode["tsInputModeMenuIconFileKey"] as? String,
        !icon.isEmpty, icon.utf8.count <= 256,
        !icon.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains),
        !icon.contains("/"), !icon.contains("\\"), icon != ".", icon != ".."
      else { throw InstallCodeError.rejected("input_source_contract_mismatch") }
      icons[key] = URL(fileURLWithPath: request.exact_bundle_path)
        .appendingPathComponent("Contents/Resources").appendingPathComponent(icon)
    }
    return .init(expectation: request, icons: icons)
  }
}

// 仅真正来自只读Apple资源映射的完整布局数据可建立fallback能力。复制相同字节到heap不通过。
private func verifyAppleLayout(_ source: TISInputSource) throws -> Bool {
  guard
    ["com.apple.keylayout.ABC", "com.apple.keylayout.US"].contains(
      try sourceString(source, kTISPropertyInputSourceID)),
    try sourceString(source, kTISPropertyBundleID) == "com.apple.keyboardlayout.all",
    try sourceString(source, kTISPropertyInputSourceType) == kTISTypeKeyboardLayout as String,
    try sourceBool(source, kTISPropertyInputSourceIsASCIICapable),
    let pointer = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData)
  else { return false }
  let object = Unmanaged<CFTypeRef>.fromOpaque(pointer).takeUnretainedValue()
  guard CFGetTypeID(object) == CFDataGetTypeID() else { return false }
  let data = unsafeBitCast(object, to: CFData.self)
  let count = CFDataGetLength(data)
  guard count >= 64, count <= 1_048_576, let bytes = CFDataGetBytePtr(data) else { return false }
  let address = UInt64(UInt(bitPattern: bytes))
  var region = proc_regionwithpathinfo()
  guard
    proc_pidinfo(
      getpid(), PROC_PIDREGIONPATHINFO, address, &region,
      Int32(MemoryLayout<proc_regionwithpathinfo>.size))
      == MemoryLayout<proc_regionwithpathinfo>.size,
    address >= region.prp_prinfo.pri_address,
    address - region.prp_prinfo.pri_address <= region.prp_prinfo.pri_size,
    UInt64(count) <= region.prp_prinfo.pri_size - (address - region.prp_prinfo.pri_address)
  else { return false }
  let path = withUnsafePointer(to: &region.prp_vip.vip_path) {
    $0.withMemoryRebound(to: CChar.self, capacity: Int(MAXPATHLEN)) { String(cString: $0) }
  }
  guard path == appleLayoutRoot + "/Contents/Resources/AppleKeyboardLayouts-L.dat" else {
    return false
  }
  var component = URL(fileURLWithPath: "/")
  for name in path.split(separator: "/") {
    component.appendPathComponent(String(name))
    var metadata = stat()
    guard lstat(component.path, &metadata) == 0, metadata.st_uid == 0,
      metadata.st_mode & S_IFMT != S_IFLNK, metadata.st_mode & 0o022 == 0
    else { return false }
  }
  var code: SecStaticCode?
  var requirement: SecRequirement?
  guard
    SecStaticCodeCreateWithPath(URL(fileURLWithPath: appleLayoutRoot) as CFURL, [], &code)
      == errSecSuccess,
    let code,
    SecRequirementCreateWithString(
      "anchor apple and identifier \"com.apple.keyboardlayout.all\"" as CFString, [], &requirement)
      == errSecSuccess,
    let requirement,
    SecStaticCodeCheckValidity(code, SecCSFlags(rawValue: kSecCSStrictValidate), requirement)
      == errSecSuccess
  else { return false }
  let handle = try FileHandle(forReadingFrom: URL(fileURLWithPath: path))
  defer { try? handle.close() }
  let size = try handle.seekToEnd()
  guard size <= 33_554_432 else { return false }
  let relative = address - region.prp_prinfo.pri_address
  guard region.prp_prinfo.pri_offset <= UInt64.max - relative else { return false }
  let offset = region.prp_prinfo.pri_offset + relative
  guard offset <= size, UInt64(count) <= size - offset else { return false }
  try handle.seek(toOffset: offset)
  return try handle.read(upToCount: count) == Data(bytes: bytes, count: count)
}
private final class CarbonInputSourceBackend: InputSourceBackend {
  var contract: SourceContract
  let notices = SourceNotifications()
  private var retained: [String: TISInputSource] = [:]
  init(contract: SourceContract) { self.contract = contract }
  func now() -> UInt64 { UInt64(ProcessInfo.processInfo.systemUptime * 1000) }
  func generation() -> UInt64 { notices.generation() }
  private func all() throws -> [TISInputSource] {
    guard Thread.isMainThread,
      let result = TISCreateInputSourceList(nil, true)?.takeRetainedValue() as? [TISInputSource],
      result.count <= 4096
    else { throw InstallCodeError.rejected("input_source_inventory_unavailable") }
    return result
  }
  private func descriptor(_ source: TISInputSource, all: [TISInputSource]) throws
    -> InputSourceDescriptor
  {
    guard let id = try sourceString(source, kTISPropertyInputSourceID), id.utf8.count <= 256,
      try sourceString(source, kTISPropertyInputSourceCategory) == kTISCategoryKeyboardInputSource
        as String,
      try all.filter({ try sourceString($0, kTISPropertyInputSourceID) == id }).count == 1
    else { throw InstallCodeError.rejected("input_source_ambiguous_identity") }
    let bundle = try sourceString(source, kTISPropertyBundleID)
    let family =
      bundle == contract.expectation.bundle_id || id == contract.expectation.bundle_id
      || contract.icons[id] != nil
    if family {
      guard bundle == contract.expectation.bundle_id, let icon = contract.icons[id],
        try sourceURL(source) == icon,
        try sourceString(source, kTISPropertyInputSourceType) == kTISTypeKeyboardInputMode as String
      else { throw InstallCodeError.rejected("input_source_target_unverifiable") }
    }
    let token: String
    if let found = retained.first(where: { CFEqual($0.value, source) }) {
      token = found.key
    } else {
      guard retained.count < 4096 else { throw InstallCodeError.rejected("input_source_budget") }
      token = UUID().uuidString
      retained[token] = source
    }
    return .init(
      id: id, token: token, target: family,
      verifiedApple: try !family && verifyAppleLayout(source),
      enabled: try sourceBool(source, kTISPropertyInputSourceIsEnabled),
      selectable: try sourceBool(source, kTISPropertyInputSourceIsSelectCapable))
  }
  func current() throws -> InputSourceDescriptor {
    let inventory = try all()
    guard let current = TISCopyCurrentKeyboardInputSource()?.takeRetainedValue() else {
      throw InstallCodeError.rejected("input_source_current_unavailable")
    }
    return try descriptor(current, all: inventory)
  }
  func fallback() throws -> InputSourceDescriptor {
    let inventory = try all()
    for id in ["com.apple.keylayout.ABC", "com.apple.keylayout.US"] {
      for source in inventory where try sourceString(source, kTISPropertyInputSourceID) == id {
        let candidate = try descriptor(source, all: inventory)
        if candidate.verifiedApple && candidate.enabled && candidate.selectable { return candidate }
      }
    }
    throw InstallCodeError.rejected("no_verified_fallback")
  }
  func original(_ id: String) throws -> InputSourceDescriptor {
    let inventory = try all()
    guard
      let source = try inventory.first(where: {
        try sourceString($0, kTISPropertyInputSourceID) == id
      })
    else {
      throw InstallCodeError.rejected("original_source_unavailable")
    }
    return try descriptor(source, all: inventory)
  }
  func select(
    _ source: InputSourceDescriptor, expecting: InputSourceDescriptor, generation: UInt64,
    authorize: () -> Bool
  ) throws {
    guard Thread.isMainThread, let retained = retained[source.token] else {
      throw InstallCodeError.rejected("input_source_invalid_handle")
    }
    let fresh = try descriptor(retained, all: all())
    guard fresh == source, source.enabled, source.selectable else {
      throw InstallCodeError.rejected("input_source_observation_changed")
    }
    // descriptor/签名验证可能很慢；在唯一真实效应边界再核选择、版本和维护期限。
    guard try current() == expecting, self.generation() == generation, authorize() else {
      throw InstallCodeError.rejected("input_source_effect_revoked")
    }
    // 没有enable/register API；只选择已启用的准确引用。
    let status = TISSelectInputSource(retained)
    guard status == noErr else {
      throw InstallCodeError.rejected("input_source_select_failed", status)
    }
  }
}
private struct NativeSourceRequest: Codable {
  let ime: InstallCodeRequest
  let leaseID: String
  let epoch: String
  let maxAgeMS: UInt64
  enum CodingKeys: String, CodingKey {
    case ime, epoch
    case leaseID = "lease_id"
    case maxAgeMS = "max_age_ms"
  }
}
private struct NativeSourceReply<T: Codable>: Codable {
  let ok: Bool
  let value: T?
  let code: String?
}
private func sourceReply<T: Codable>(_ work: () throws -> T) -> UnsafeMutablePointer<CChar>? {
  let reply: NativeSourceReply<T>
  do { reply = .init(ok: true, value: try work(), code: nil) } catch InstallCodeError.rejected(
    let code, _)
  { reply = .init(ok: false, value: nil, code: code) } catch {
    reply = .init(ok: false, value: nil, code: "input_source_backend_failed")
  }
  guard let raw = try? installCanonical(reply), let value = String(data: raw, encoding: .utf8)
  else { return nil }
  return strdup(value)
}
private func decodeSourceRequest(_ bytes: UnsafePointer<UInt8>?, _ count: UInt) throws
  -> NativeSourceRequest
{
  guard let bytes, count > 0, count <= 32_768 else {
    throw InstallCodeError.rejected("invalid_request")
  }
  let raw = Data(bytes: bytes, count: Int(count))
  let request = try JSONDecoder().decode(NativeSourceRequest.self, from: raw)
  guard try installCanonical(request) == raw, UUID(uuidString: request.leaseID) != nil,
    UUID(uuidString: request.epoch) != nil, request.maxAgeMS > 0, request.maxAgeMS <= 120_000
  else { throw InstallCodeError.rejected("invalid_request") }
  return request
}
private final class NativeInputSourceLease {
  let request: NativeSourceRequest
  let backend: CarbonInputSourceBackend
  let machine: InputSourceMachine
  init(_ request: NativeSourceRequest) throws {
    guard Thread.isMainThread else {
      throw InstallCodeError.rejected("input_source_main_thread_required")
    }
    let slot = try InputSourceSlot.acquire()
    self.request = request
    backend = CarbonInputSourceBackend(contract: try SourceContract.verified(request.ime))
    machine = try InputSourceMachine(
      backend: backend, deadline: backend.now() + request.maxAgeMS, slot: slot)
  }
}
@_cdecl("iuis_input_source_prepare")
public func iuisInputSourcePrepare(
  _ bytes: UnsafePointer<UInt8>?, _ count: UInt,
  _ output: UnsafeMutablePointer<UnsafeMutableRawPointer?>?
) -> UnsafeMutablePointer<CChar>? {
  output?.pointee = nil
  var lease: NativeInputSourceLease?
  let result = sourceReply { () throws -> InputSourceObservation in
    guard output != nil else { throw InstallCodeError.rejected("invalid_request") }
    let made = try NativeInputSourceLease(decodeSourceRequest(bytes, count))
    lease = made
    return made.machine.observation
  }
  if result != nil, let lease { output?.pointee = Unmanaged.passRetained(lease).toOpaque() }
  return result
}
@_cdecl("iuis_input_source_action")
public func iuisInputSourceAction(
  _ handle: UnsafeMutableRawPointer?, _ action: UInt32,
  _ bytes: UnsafePointer<UInt8>?, _ count: UInt, _ check: WriterEffectCheck?,
  _ context: UnsafeMutableRawPointer?
) -> UnsafeMutablePointer<CChar>? {
  sourceReply {
    guard Thread.isMainThread, let handle, let check, let context else {
      throw InstallCodeError.rejected("invalid_request")
    }
    let lease = Unmanaged<NativeInputSourceLease>.fromOpaque(handle).takeUnretainedValue()
    let authorize = { check(context) == 0 }
    switch action {
    case 1: return lease.machine.detach(authorize: authorize)
    case 2, 3:
      let request = try decodeSourceRequest(bytes, count)
      guard request.leaseID == lease.request.leaseID, request.epoch == lease.request.epoch,
        request.maxAgeMS == lease.request.maxAgeMS,
        request.ime.subject == lease.request.ime.subject,
        request.ime.bundle_id == lease.request.ime.bundle_id,
        request.ime.exact_bundle_path == lease.request.ime.exact_bundle_path
      else { throw InstallCodeError.rejected("input_source_rebind_mismatch") }
      lease.backend.contract = try SourceContract.verified(request.ime)
      if action == 2 { return lease.machine.assertDetached(authorize: authorize) }
      return lease.machine.restore(authorize: authorize)
    default: throw InstallCodeError.rejected("invalid_request")
    }
  }
}
@_cdecl("iuis_input_source_free")
public func iuisInputSourceFree(_ handle: UnsafeMutableRawPointer?) {
  if let handle { Unmanaged<NativeInputSourceLease>.fromOpaque(handle).release() }
}
