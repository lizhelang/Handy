import Foundation
import Darwin

struct InputiaPairBinding: Codable, Equatable {
  let product_id: String
  let installation_id: String
  let pair_release_id: String
}

struct InputiaLocatedInstallation: Decodable, Equatable {
  struct Receipt: Decodable, Equatable {
    struct Paths: Decodable, Equatable { let control: String; let ime: String; let settings: String }
    struct DataLocation: Decodable, Equatable { let kind: String; let run_id: String? }
    let product_id: String
    let installation_id: String
    let profile_id: String
    let release_id: String
    let components: Paths
    let data: DataLocation
  }
  let receipt: Receipt
  let handy_root: String
  let inputia_root: String
  let pair_manifest: String
  var binding: InputiaPairBinding {
    .init(product_id: receipt.product_id, installation_id: receipt.installation_id,
          pair_release_id: receipt.release_id)
  }
}

#if INPUTIA_RELEASE_PAIR_V2
@_silgen_name("inputia_installation_load")
private func installationLoad(_ context: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_string_free")
private func installationStringFree(_ value: UnsafeMutablePointer<CChar>?)
#endif

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
  var installation: InputiaLocatedInstallation? = nil

  var profileID: String { installation?.receipt.profile_id ?? runID.map { "unified-candidate:\($0)" } ?? "handy-local" }
  var pairManifestURL: URL { installation.map { URL(fileURLWithPath: $0.pair_manifest) }
    ?? root.deletingLastPathComponent().appendingPathComponent("pair-manifest.json") }
  var pairBinding: InputiaPairBinding? { installation?.binding }

  func readPairManifest() throws -> Data {
    try Self.readBoundedFile(pairManifestURL, limit: 16_384, privateFile: installation != nil)
  }

  func readEndpoint() throws -> Data {
    try Self.readBoundedFile(handyRoot.appendingPathComponent("integration-endpoint.json"), limit: 4096, privateFile: true)
  }

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
      #if INPUTIA_RELEASE_PAIR_V2
      let profile = try loadReleaseInstallation()
      #else
      let profile = try resolve(
        bundleIdentifier: Bundle.main.bundleIdentifier,
        info: Bundle.main.infoDictionary ?? [:],
        environment: ProcessInfo.processInfo.environment
      )
      #endif
      try profile.validateCandidatePaths()
      return profile
    } catch {
      NSLog("Inputia profile rejected: %@", String(describing: error))
      exit(78)
    }
  }()

  #if INPUTIA_RELEASE_PAIR_V2
  private static func loadReleaseInstallation() throws -> Self {
    guard ProcessInfo.processInfo.environment["INPUTIA_PROFILE_RUN_ID"] == nil else {
      throw InputiaProfileError.conflictingRunID
    }
    let trust = InputiaEmbeddedPairTrust.trust
    let context: [String: Any] = ["product_id": trust.productID, "release_id": trust.releaseID,
      "uid": geteuid(), "home": NSHomeDirectory()]
    let data = try JSONSerialization.data(withJSONObject: context)
    guard let json = String(data: data, encoding: .utf8),
      let raw = json.withCString({ installationLoad($0) }) else { throw InputiaProfileError.unauthorizedCandidate }
    defer { installationStringFree(raw) }
    struct Reply: Decodable { let ok: Bool; let installation: InputiaLocatedInstallation? }
    let reply = try JSONDecoder().decode(Reply.self, from: Data(String(cString: raw).utf8))
    guard reply.ok, let installation = reply.installation else { throw InputiaProfileError.unauthorizedCandidate }
    #if INPUTIA_SETTINGS_LAUNCHER
    let expectedPath = installation.receipt.components.settings
    #else
    let expectedPath = installation.receipt.components.ime
    #endif
    guard Bundle.main.bundleURL.path == expectedPath else { throw InputiaProfileError.pathOutsideProfile }
    return Self(isCandidate: true, runID: installation.receipt.data.run_id,
      root: URL(fileURLWithPath: installation.inputia_root, isDirectory: true),
      handyRoot: URL(fileURLWithPath: installation.handy_root, isDirectory: true), installation: installation)
  }
  #endif

  /// 逐层固定目录句柄，拒绝链接、跨用户文件及无界读取；不把发现文件当身份认证。
  static func readBoundedFile(_ url: URL, limit: Int, privateFile: Bool) throws -> Data {
    let path = url.path
    guard path.hasPrefix("/"), url.standardizedFileURL.path == path,
      !path.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) else {
      throw InputiaProfileError.pathOutsideProfile
    }
    var parent = Darwin.open("/", O_RDONLY | O_DIRECTORY | O_CLOEXEC)
    guard parent >= 0 else { throw InputiaProfileError.pathInspectionFailed }
    defer { Darwin.close(parent) }
    let parts = path.split(separator: "/")
    for (index, part) in parts.enumerated() {
      let last = index == parts.count - 1
      let flags = O_RDONLY | O_CLOEXEC | O_NOFOLLOW | (last ? O_NONBLOCK : O_DIRECTORY)
      let descriptor = String(part).withCString { Darwin.openat(parent, $0, flags) }
      guard descriptor >= 0 else { throw InputiaProfileError.pathInspectionFailed }
      var metadata = stat()
      guard fstat(descriptor, &metadata) == 0 else { Darwin.close(descriptor); throw InputiaProfileError.pathInspectionFailed }
      if !last {
        let owner = metadata.st_uid == 0 || metadata.st_uid == geteuid()
        let writable = metadata.st_mode & 0o022 != 0
        let rootSticky = metadata.st_uid == 0 && metadata.st_mode & 0o1000 != 0
        guard metadata.st_mode & S_IFMT == S_IFDIR, owner, !writable || rootSticky else {
          Darwin.close(descriptor); throw InputiaProfileError.invalidProfileDirectory
        }
        Darwin.close(parent); parent = descriptor
        continue
      }
      defer { Darwin.close(descriptor) }
      guard metadata.st_mode & S_IFMT == S_IFREG, metadata.st_nlink == 1,
        metadata.st_uid == geteuid(), metadata.st_mode & (privateFile ? 0o077 : 0o022) == 0,
        metadata.st_size >= 0, metadata.st_size <= limit else { throw InputiaProfileError.pathInspectionFailed }
      var bytes = [UInt8](repeating: 0, count: limit + 1)
      var count = 0
      while count < bytes.count {
        let amount = bytes.withUnsafeMutableBytes { Darwin.read(descriptor, $0.baseAddress!.advanced(by: count), $0.count - count) }
        if amount == 0 { break }
        if amount < 0 { if errno == EINTR { continue }; throw InputiaProfileError.pathInspectionFailed }
        count += amount
      }
      guard count <= limit else { throw InputiaProfileError.pathInspectionFailed }
      return Data(bytes.prefix(count))
    }
    throw InputiaProfileError.pathInspectionFailed
  }

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
