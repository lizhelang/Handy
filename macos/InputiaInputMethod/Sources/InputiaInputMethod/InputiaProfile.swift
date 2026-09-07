import Foundation
import Darwin

enum InputiaProfileError: Error, Equatable {
  case unauthorizedCandidate
  case missingRunID
  case invalidRunID
  case conflictingRunID
  case pathOutsideProfile
  case symbolicLink
  case hardLink
  case pathInspectionFailed
  case invalidProfileDirectory
}

/// 候选进程只能使用明确配对的合成 profile；解析失败不能回退到日常目录。
struct InputiaProfile: Equatable {
  let isCandidate: Bool
  let runID: String?
  let root: URL
  let handyRoot: URL

  var settings: URL { root.appendingPathComponent("settings.json") }
  var memory: URL { root.appendingPathComponent("inputia_memory.db") }
  var rime: URL { root.appendingPathComponent("rime", isDirectory: true) }
  var outbox: URL { root.appendingPathComponent("outbox.db") }
  var snapshots: URL { root.appendingPathComponent("snapshots", isDirectory: true) }
  var logs: URL { root.appendingPathComponent("logs", isDirectory: true) }
  var policy: URL { root.appendingPathComponent("policy.db") }
  var writablePaths: [URL] {
    [settings, memory, rime, rime.appendingPathComponent("build"),
     rime.appendingPathComponent("sync"), outbox, snapshots, logs, policy]
  }

  static let current: InputiaProfile = {
    do {
      #if INPUTIA_UNIFIED_CANDIDATE
      #if INPUTIA_SETTINGS_LAUNCHER
      let expectedCandidate = "com.inputia.settings.UnifiedCandidate"
      #else
      let expectedCandidate = "com.inputia.inputmethod.Inputia.UnifiedCandidate"
      #endif
      #else
      let expectedCandidate: String? = nil
      #endif
      try validateCompiledIdentity(bundleIdentifier: Bundle.main.bundleIdentifier,
                                   expectedCandidate: expectedCandidate)
      let profile = try resolve(
        bundleIdentifier: Bundle.main.bundleIdentifier,
        info: Bundle.main.infoDictionary ?? [:],
        environment: ProcessInfo.processInfo.environment
      )
      try profile.validateCandidatePaths()
      return profile
    } catch {
      NSLog("Inputia profile rejected: %@", String(describing: error))
      exit(78)
    }
  }()

  static func validateCompiledIdentity(bundleIdentifier: String?, expectedCandidate: String?) throws {
    if let expectedCandidate {
      guard bundleIdentifier == expectedCandidate else { throw InputiaProfileError.unauthorizedCandidate }
    } else if bundleIdentifier?.hasSuffix(".UnifiedCandidate") == true {
      throw InputiaProfileError.unauthorizedCandidate
    }
  }

  static func resolve(
    bundleIdentifier: String?,
    info: [String: Any],
    environment: [String: String],
    applicationSupport: URL? = nil
  ) throws -> Self {
    let base = applicationSupport
      ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
      ?? URL(fileURLWithPath: NSTemporaryDirectory())
    let suffixMatches = bundleIdentifier?.hasSuffix(".UnifiedCandidate") == true
    let marker = info["InputiaDevelopmentCandidate"] as? NSNumber
    let marked = marker.map { CFGetTypeID($0) == CFBooleanGetTypeID() && $0.boolValue } ?? false
    let environmentID = environment["INPUTIA_PROFILE_RUN_ID"]
    let plistID = info["InputiaProfileRunID"] as? String
    if info["InputiaProfileRunID"] != nil, plistID == nil { throw InputiaProfileError.invalidRunID }
    guard suffixMatches && marked else {
      guard !suffixMatches, !marked, environmentID == nil, info["InputiaProfileRunID"] == nil else {
        throw InputiaProfileError.unauthorizedCandidate
      }
      return Self(isCandidate: false, runID: nil,
                  root: base.appendingPathComponent("Inputia", isDirectory: true),
                  handyRoot: base.appendingPathComponent("com.pais.handy", isDirectory: true))
    }
    if let environmentID, let plistID, environmentID != plistID {
      throw InputiaProfileError.conflictingRunID
    }
    guard let runID = environmentID ?? plistID else { throw InputiaProfileError.missingRunID }
    // 严格 ASCII 单段标识，不接受点、空白、转义、分隔符或超长路径。
    let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_")
    guard (1...64).contains(runID.utf8.count),
          runID.unicodeScalars.allSatisfy({ allowed.contains($0) }) else {
      throw InputiaProfileError.invalidRunID
    }
    let runRoot = base.appendingPathComponent("HandyUnifiedCandidate", isDirectory: true)
      .appendingPathComponent(runID, isDirectory: true)
    return Self(isCandidate: true, runID: runID,
                root: runRoot.appendingPathComponent("Inputia", isDirectory: true),
                handyRoot: runRoot.appendingPathComponent("Handy", isDirectory: true))
  }

  func validateCandidatePaths() throws {
    guard isCandidate else { return }
    try validateDirectPaths()
    // 只在启动/重新初始化时遍历候选目录元数据，不读取文件正文，不进入链接目标。
    try Self.auditExistingTree(root)
    try Self.auditExistingTree(handyRoot)
  }

  func validateSettingsPath(_ path: String) throws {
    guard isCandidate else { return }
    guard URL(fileURLWithPath: path).standardizedFileURL == settings.standardizedFileURL else {
      throw InputiaProfileError.pathOutsideProfile
    }
    // 此方法也由频繁的设置读取调用，不能在按键路径遍历 Rime/历史目录。
    try validateDirectPaths()
  }

  func allowsHandyImport(_ path: String) -> Bool {
    guard isCandidate else { return true }
    let url = URL(fileURLWithPath: path).standardizedFileURL
    let expected = ["history.db", "clipboard.db"].map { handyRoot.appendingPathComponent($0).standardizedFileURL }
    guard expected.contains(url) else { return false }
    return (try? Self.validateDatabasePath(url)) != nil
  }

  func isolatedSettings(_ original: [String: Any], settingsPath: String) throws -> [String: Any] {
    guard isCandidate else { return original }
    try validateSettingsPath(settingsPath)
    var result = original
    result["rime_user_data_dir"] = rime.path
    result["memory_db_path"] = memory.path
    result["integration_outbox_path"] = outbox.path
    result["integration_snapshot_dir"] = snapshots.path
    result["integration_policy_path"] = policy.path
    result["integration_log_dir"] = logs.path
    result["integration_profile_run_id"] = runID
    return result
  }

  private func validateDirectPaths() throws {
    for path in writablePaths + [handyRoot, handyRoot.appendingPathComponent("history.db"),
                                 handyRoot.appendingPathComponent("clipboard.db")] {
      if path.pathExtension == "db" {
        try Self.validateDatabasePath(path)
      } else {
        try Self.rejectLinks(in: path)
      }
    }
  }

  private static func validateDatabasePath(_ url: URL) throws {
    for suffix in ["", "-wal", "-shm", "-journal"] {
      try rejectLinks(in: URL(fileURLWithPath: url.path + suffix))
    }
  }

  /// lstat 只读取链接自身；只有不存在可放行，权限/IO 等错误不能伪装为空目录。
  private static func inspect(_ url: URL) throws -> stat? {
    var metadata = stat()
    let result = url.path.withCString { Darwin.lstat($0, &metadata) }
    guard result == 0 else {
      if errno == ENOENT { return nil }
      throw InputiaProfileError.pathInspectionFailed
    }
    let kind = metadata.st_mode & mode_t(S_IFMT)
    if kind == mode_t(S_IFLNK) { throw InputiaProfileError.symbolicLink }
    if kind == mode_t(S_IFREG), metadata.st_nlink > 1 { throw InputiaProfileError.hardLink }
    return metadata
  }

  private static func rejectLinks(in url: URL) throws {
    var cursor = url.standardizedFileURL
    var ancestors: [URL] = []
    while cursor.path != "/" {
      ancestors.append(cursor)
      cursor.deleteLastPathComponent()
    }
    // 从根往下验证，不能先 lstat 一个含链接祖先的后代路径。
    for ancestor in ancestors.reversed() { _ = try inspect(ancestor) }
  }

  private static func auditExistingTree(_ root: URL) throws {
    guard let metadata = try inspect(root) else { return }
    guard metadata.st_mode & mode_t(S_IFMT) == mode_t(S_IFDIR) else {
      throw InputiaProfileError.invalidProfileDirectory
    }
    var pending = [root]
    while let directory = pending.popLast() {
      // 再检查待访问目录。此启动审计不宣称抵御同 UID 进程实时换路径的完整沙箱。
      guard let current = try inspect(directory) else { continue }
      guard current.st_mode & mode_t(S_IFMT) == mode_t(S_IFDIR) else {
        throw InputiaProfileError.invalidProfileDirectory
      }
      let children = try FileManager.default.contentsOfDirectory(
        at: directory, includingPropertiesForKeys: nil, options: []
      )
      for child in children {
        guard let childMetadata = try inspect(child) else { continue }
        if childMetadata.st_mode & mode_t(S_IFMT) == mode_t(S_IFDIR) {
          pending.append(child)
        }
      }
    }
  }
}
