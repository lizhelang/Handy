import Foundation

@_silgen_name("inputia_settings_request")
private func settingsRequest(_ request: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_string_free")
private func settingsStringFree(_ pointer: UnsafeMutablePointer<CChar>?)
@_silgen_name("inputia_session_new_from_settings_snapshot")
private func settingsSnapshotSession(_ request: UnsafePointer<CChar>) -> UnsafeMutableRawPointer?
@_silgen_name("inputia_session_settings_applied")
private func settingsSessionApplied(_ session: UnsafeMutableRawPointer, _ request: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_settings_flush_applications")
private func settingsFlushApplications(_ request: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

/// 设置窗口与输入法共用无会话 C ABI。保存成功只表示落盘，生效需要 Host 回执。
enum InputiaSettingsStore {
  struct Failure: LocalizedError {
    let code: String
    var errorDescription: String? {
      switch code {
      case "busy": return "设置正在被另一个窗口保存，请稍后重试"
      case "maintenance": return "应用正在维护，设置暂不可修改"
      case "external_edit": return "检测到外部修改，请检查并导入修改后的设置"
      case "external_changed": return "预览后文件再次改变，请重新查看外部修改"
      case "repair_required", "invalid_document": return "设置文件需要修复，原文件已保留"
      case "commit_uncertain": return "尚未确认保存结果，请用同一操作重试"
      case "conflict": return "另一处已修改设置，已显示当前值，请重新选择"
      case "outcome_expired": return "操作回执已过期，已显示当前值，请重新选择"
      case "runtime_path_mismatch": return "设置的数据路径与当前配置不一致，输入会话未切换"
      default: return "设置操作失败（\(code)）"
      }
    }
  }
  struct Snapshot {
    let storeID: String
    let revision: String
    let digest: String
    let values: [String: Any]
    init(_ raw: [String: Any]) throws {
      guard let storeID = raw["store_id"] as? String, UUID(uuidString: storeID) != nil,
        let revision = raw["revision"] as? String, UInt64(revision)?.description == revision,
        let digest = raw["values_digest"] as? String, digest.utf8.count == 64,
        digest.utf8.allSatisfy({ (48...57).contains($0) || (97...102).contains($0) }),
        let values = raw["values"] as? [String: Any] else { throw Failure(code: "invalid_response") }
      self.storeID = storeID; self.revision = revision; self.digest = digest; self.values = values
    }
    var identity: String { "\(storeID):\(revision):\(digest)" }
    func decode<T: Decodable>(_ type: T.Type) throws -> T {
      try JSONDecoder().decode(type, from: JSONSerialization.data(withJSONObject: values))
    }
  }
  struct Operation {
    let raw: [String: Any]
    init(base: Snapshot, patch: [String: Any]) {
      raw = ["operation_id": "v1:\(base.storeID):\(base.revision):\(UUID().uuidString.lowercased())",
             "expected_store_id": base.storeID, "expected_revision": base.revision, "patch": patch]
    }
  }
  struct Applied {
    let status: String
    let current: Snapshot
    let commitRevision: String?
  }
  struct External {
    let storeID: String
    let revision: String
    let observedFileDigest: String
    let values: [String: Any]
    init(_ raw: [String: Any]) throws {
      guard let digest = raw["observed_file_digest"] as? String else { throw Failure(code: "invalid_response") }
      let validated = try Snapshot(["store_id":raw["store_id"] ?? NSNull(), "revision":raw["revision"] ?? NSNull(),
        "values_digest":digest, "values":raw["values"] ?? NSNull()])
      storeID = validated.storeID; revision = validated.revision; observedFileDigest = digest; values = validated.values
    }
  }
  struct ImportOperation {
    let raw: [String: Any]
    init(_ external: External) {
      raw = ["operation_id":"v1:\(external.storeID):\(external.revision):\(UUID().uuidString.lowercased())",
        "expected_store_id":external.storeID,"expected_revision":external.revision,
        "observed_file_digest":external.observedFileDigest]
    }
  }
  struct ApplicationStatus {
    let storeID: String
    let revision: String
    let digest: String
    let sessions: [[String: Any]]
    init(_ raw: [String: Any]) throws {
      guard raw["scope"] as? String == "observed_engine_sessions", raw["lease_ms"] as? Int == 2500,
        let store = raw["current_store_id"] as? String, let revision = raw["current_revision"] as? String,
        let digest = raw["current_values_digest"] as? String, let sessions = raw["sessions"] as? [[String: Any]],
        sessions.allSatisfy({ $0["instance_id"] is String && $0["store_id"] is String && $0["revision"] is String
          && $0["values_digest"] is String && $0["applied_fields"] is [String] && $0["unavailable_fields"] is [String] }) else {
        throw Failure(code: "invalid_response")
      }
      self.storeID = store; self.revision = revision; self.digest = digest; self.sessions = sessions
    }
    func summary(for snapshot: Snapshot) -> String {
      guard storeID == snapshot.storeID, revision == snapshot.revision, digest == snapshot.digest else {
        return "已保存，等待当前版本的输入会话确认"
      }
      let matching = sessions.filter { $0["store_id"] as? String == storeID && $0["revision"] as? String == revision
        && $0["values_digest"] as? String == digest }
      guard !matching.isEmpty else { return "已保存版本 \(revision)，等待输入会话确认" }
      let degraded = matching.contains { !($0["unavailable_fields"] as? [String] ?? []).isEmpty }
      let suffix = matching.count != sessions.count ? "；仍有旧版本会话等待切换" : (degraded ? "；部分功能降级" : "")
      let fontPending = matching.contains { !($0["applied_fields"] as? [String] ?? []).contains("candidate_font_size") }
      let required = InputiaSettingsStore.editableKeys.subtracting(["menu_icon_variant"])
      let pending = matching.contains { !required.isSubset(of: Set($0["applied_fields"] as? [String] ?? [])) }
      return "收到 \(matching.count) 个输入会话对版本 \(revision) 的确认" + suffix
        + (pending ? "；部分设置待生效" : "") + (fontPending ? "；候选字体待确认" : "") + "；菜单图标需重启/重装"
    }
  }
  /// 编辑器只允许发送用户能操作的字段，运行时资源和未知扩展字段不在此列表。
  static let editableKeys: Set<String> = ["schema_id", "candidate_page_size", "candidate_font_size",
    "menu_icon_variant", "shift_toggle_enabled", "input_mode_toggle_shortcut", "chinese_script",
    "script_toggle_shortcut", "punctuation_preference", "character_width_preference",
    "spelling_correction_enabled", "memory_enabled", "privacy_learning_enabled", "sensitive_bundle_ids"]
  static func dirtyPatch<T: Encodable>(from base: T, to next: T) throws -> [String: Any] {
    let old = try JSONSerialization.jsonObject(with: JSONEncoder().encode(base)) as? [String: Any] ?? [:]
    let new = try JSONSerialization.jsonObject(with: JSONEncoder().encode(next)) as? [String: Any] ?? [:]
    return new.filter { key, value in
      editableKeys.contains(key) && !NSDictionary(dictionary: ["value": old[key] ?? NSNull()]).isEqual(to: ["value": value])
    }
  }
  static func validateRuntimePaths(_ snapshot: Snapshot, expected: [String: String], required: Set<String>) throws {
    for (key, path) in expected {
      let value = snapshot.values[key]
      if value == nil || value is NSNull {
        if required.contains(key) { throw Failure(code: "runtime_path_mismatch") }
      } else if value as? String != path { throw Failure(code: "runtime_path_mismatch") }
    }
  }
  private static func call(_ request: [String: Any]) throws -> [String: Any] {
    let data = try JSONSerialization.data(withJSONObject: request, options: [.sortedKeys])
    guard let json = String(data: data, encoding: .utf8),
      let raw = json.withCString({ settingsRequest($0) }) else { throw Failure(code: "storage_unavailable") }
    defer { settingsStringFree(raw) }
    guard let reply = try JSONSerialization.jsonObject(with: Data(String(cString: raw).utf8)) as? [String: Any],
      reply["ok"] as? Bool == true else {
      let reply = try? JSONSerialization.jsonObject(with: Data(String(cString: raw).utf8)) as? [String: Any]
      throw Failure(code: reply?["code"] as? String ?? "invalid_response")
    }
    return reply
  }
  static func read(path: String) throws -> Snapshot {
    let reply = try call(["action": "read", "path": path])
    guard let raw = reply["snapshot"] as? [String: Any] else { throw Failure(code: "invalid_response") }
    return try Snapshot(raw)
  }
  static func apply(path: String, operation: Operation) throws -> Applied {
    // 不确定结果只重用同一 ID 与参数，不重新计算 toggle。最终失败由调用方保留操作。
    var reply: [String: Any] = [:]
    for attempt in 0..<2 {
      do { reply = try call(["action": "apply", "path": path, "request": operation.raw]); break }
      catch let failure as Failure where failure.code == "commit_uncertain" && attempt == 0 { continue }
    }
    return try applied(reply)
  }
  private static func applied(_ reply: [String: Any]) throws -> Applied {
    guard let result = reply["result"] as? [String: Any], let status = result["status"] as? String,
      ["saved", "conflict", "outcome_expired"].contains(status),
      let current = result["current"] as? [String: Any] else { throw Failure(code: "invalid_response") }
    return Applied(status: status, current: try Snapshot(current), commitRevision: result["commit_revision"] as? String)
  }
  static func inspectExternal(path: String) throws -> External {
    let reply = try call(["action":"inspect_external", "path":path])
    guard let external = reply["external"] as? [String: Any] else { throw Failure(code: "invalid_response") }
    return try External(external)
  }
  static func importExternal(path: String, operation: ImportOperation) throws -> Applied {
    for attempt in 0..<2 {
      do { return try applied(call(["action":"import_external", "path":path, "request":operation.raw])) }
      catch let failure as Failure where failure.code == "commit_uncertain" && attempt == 0 { continue }
    }
    throw Failure(code: "commit_uncertain")
  }
  static func applicationStatus(path: String) throws -> ApplicationStatus {
    let reply = try call(["action":"application_status", "path":path])
    guard let value = reply["application"] as? [String: Any] else { throw Failure(code: "invalid_response") }
    return try ApplicationStatus(value)
  }
  static func reportApplication(session: UnsafeMutableRawPointer, applied: [String], unavailable: [String] = []) -> Bool {
    guard let data = try? JSONSerialization.data(withJSONObject: ["applied_fields":applied,"unavailable_fields":unavailable]),
      let json = String(data: data, encoding: .utf8), let raw = json.withCString({ settingsSessionApplied(session, $0) }) else { return false }
    defer { settingsStringFree(raw) }
    let result = try? JSONSerialization.jsonObject(with: Data(String(cString: raw).utf8)) as? [String: Any]
    return result?["ok"] as? Bool == true
  }
  static func flushApplications(path: String) {
    guard let data = try? JSONSerialization.data(withJSONObject: ["path":path]), let json = String(data: data, encoding: .utf8),
      let raw = json.withCString({ settingsFlushApplications($0) }) else { return }
    settingsStringFree(raw)
  }
  static func openSession(path: String, snapshot: Snapshot, sharedData: String?, withoutMemory: Bool) -> UnsafeMutableRawPointer? {
    var request: [String: Any] = ["path": path, "store_id": snapshot.storeID, "revision": snapshot.revision,
      "values_digest": snapshot.digest, "without_memory": withoutMemory]
    request["rime_shared_data_dir"] = sharedData ?? NSNull() as Any
    guard let data = try? JSONSerialization.data(withJSONObject: request), let json = String(data: data, encoding: .utf8) else { return nil }
    return json.withCString { settingsSnapshotSession($0) }
  }
}

/// 一个窗口/快捷键保留自己的 CAS 基准。不确定提交结果不能被下一次 UI 动作改写。
final class InputiaSettingsEdit {
  private(set) var base: InputiaSettingsStore.Snapshot
  private(set) var pending: InputiaSettingsStore.Operation?
  private var everUncertain = false
  init(_ snapshot: InputiaSettingsStore.Snapshot) { base = snapshot }
  func apply(path: String, patch: [String: Any]) throws -> InputiaSettingsStore.Applied? {
    if pending == nil {
      guard !patch.isEmpty else { return nil }
      pending = .init(base: base, patch: patch)
      everUncertain = false
    }
    guard let operation = pending else { return nil }
    do {
      let result = try InputiaSettingsStore.apply(path: path, operation: operation)
      base = result.current; pending = nil; everUncertain = false
      return result
    } catch let failure as InputiaSettingsStore.Failure {
      if ["commit_uncertain", "storage_unavailable", "invalid_response"].contains(failure.code) { everUncertain = true }
      // 首次明确拒绝可重选/导入；出现过不确定提交后，锁忙不能解除同 ID 的追踪。
      if !everUncertain { pending = nil }
      throw failure
    } catch {
      everUncertain = true
      throw error
    }
  }
}

/// 逐键路径只读内存。后台协调器先完成严格读取，再公布完整 snapshot；错误不回落到宽松文件读取。
final class InputiaSettingsCache {
  struct State { let snapshot: InputiaSettingsStore.Snapshot?; let failure: String?; let application: InputiaSettingsStore.ApplicationStatus?; let generation: UInt64 }
  private static let registryLock = NSLock()
  private static var registry: [String: InputiaSettingsCache] = [:]
  static func shared(path: String) -> InputiaSettingsCache {
    registryLock.lock(); defer { registryLock.unlock() }
    if let cached = registry[path] { return cached }
    let cache = InputiaSettingsCache(path: path)
    registry[path] = cache
    return cache
  }
  private let lock = NSLock()
  private let queue = DispatchQueue(label: "com.inputia.settings.snapshot", qos: .utility)
  private let path: String
  private let read: (String) throws -> InputiaSettingsStore.Snapshot
  private var snapshot: InputiaSettingsStore.Snapshot?
  private var failure: String?
  private var application: InputiaSettingsStore.ApplicationStatus?
  private var generation: UInt64 = 0
  private var timer: DispatchSourceTimer?
  init(path: String, polling: Bool = true,
       read: @escaping (String) throws -> InputiaSettingsStore.Snapshot = InputiaSettingsStore.read) {
    self.path = path; self.read = read
    refresh()
    if polling {
      let timer = DispatchSource.makeTimerSource(queue: queue)
      timer.schedule(deadline: .now() + 0.75, repeating: 0.75)
      timer.setEventHandler { [weak self] in self?.refresh() }
      self.timer = timer; timer.resume()
    }
  }
  deinit { timer?.cancel() }
  var state: State {
    lock.lock(); defer { lock.unlock() }
    return State(snapshot: failure == nil ? snapshot : nil, failure: failure, application: application, generation: generation)
  }
  func publish(_ value: InputiaSettingsStore.Snapshot) {
    lock.lock(); defer { lock.unlock() }
    if let current = snapshot, current.storeID == value.storeID,
      let existing = UInt64(current.revision), let incoming = UInt64(value.revision), existing > incoming { return }
    snapshot = value; failure = nil
  }
  /// 只供后台预读、启动和有界诊断使用，输入按键不调用此方法。
  func refresh() {
    do {
      publish(try read(path))
      lock.lock(); generation &+= 1; lock.unlock()
      InputiaSettingsStore.flushApplications(path: path)
      let observed = try? InputiaSettingsStore.applicationStatus(path: path)
      lock.lock(); application = observed; lock.unlock()
    }
    catch {
      lock.lock(); failure = (error as? InputiaSettingsStore.Failure)?.code ?? "storage_unavailable"; lock.unlock()
    }
  }
}

/// 同版本失败只有在后台再次成功读取且退避期限到达时重试；计时不受系统时钟回拨影响。
final class InputiaSettingsRetryGate {
  private var identity: String?
  private var generation: UInt64 = 0
  private var deadline: TimeInterval = 0
  private let uptime: () -> TimeInterval
  init(uptime: @escaping () -> TimeInterval = { ProcessInfo.processInfo.systemUptime }) { self.uptime = uptime }
  func begin(identity: String, generation: UInt64) -> Bool {
    let now = uptime()
    if self.identity == identity && (self.generation == generation || now < deadline) { return false }
    self.identity = identity; self.generation = generation; deadline = now + 1.5
    return true
  }
}

enum InputiaSettingsSessionSwap {
  /// 先构造新会话；失败时原句柄仍由调用方持有，成功后才释放旧句柄。
  static func replace(_ current: inout UnsafeMutableRawPointer?,
    open: () -> UnsafeMutableRawPointer?, release: (UnsafeMutableRawPointer?) -> Void) -> Bool {
    guard let next = open() else { return false }
    let previous = current
    current = next
    release(previous)
    return true
  }
}
