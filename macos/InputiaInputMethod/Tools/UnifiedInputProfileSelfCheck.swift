import Foundation

@main
struct UnifiedInputProfileSelfCheck {
  static func main() throws {
    if CommandLine.arguments.contains("--current") {
      print("candidate=\(InputiaProfile.current.isCandidate)")
      return
    }
    let base = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Caches")
      .appendingPathComponent("InputiaProfileCheck-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: base, withIntermediateDirectories: false)
    defer { try? FileManager.default.removeItem(at: base) }
    let candidateID = "com.inputia.inputmethod.Inputia.UnifiedCandidate"
    let marked: [String: Any] = ["InputiaDevelopmentCandidate": true]
    var checks = 0
    func check(_ condition: Bool, _ label: String) {
      checks += 1
      if !condition { fputs("profile self-check failed: \(label)\n", stderr); exit(1) }
    }
    func rejects(_ expected: InputiaProfileError, _ operation: () throws -> Void) {
      do { try operation(); check(false, "expected \(expected)") }
      catch let error as InputiaProfileError { check(error == expected, "wrong error \(error)") }
      catch { check(false, "unexpected error") }
    }
    let daily = try InputiaProfile.resolve(bundleIdentifier: "com.inputia.inputmethod.Inputia", info: [:], environment: [:], applicationSupport: base)
    for identity: String? in [nil, "com.inputia.inputmethod.Inputia", "com.inputia.settings.UnifiedCandidate"] {
      rejects(.unauthorizedCandidate) { try InputiaProfile.validateCompiledIdentity(bundleIdentifier: identity, expectedCandidate: candidateID) }
    }
    rejects(.unauthorizedCandidate) { try InputiaProfile.validateCompiledIdentity(bundleIdentifier: candidateID, expectedCandidate: nil) }
    try InputiaProfile.validateCompiledIdentity(bundleIdentifier: candidateID, expectedCandidate: candidateID)
    try InputiaProfile.validateCompiledIdentity(bundleIdentifier: nil, expectedCandidate: nil)
    check(true, "compiled and bundle candidate identities agree")
    check(daily.root == base.appendingPathComponent("Inputia", isDirectory: true), "daily root retained")
    check(daily.handyRoot == base.appendingPathComponent("com.pais.handy", isDirectory: true), "daily Handy root retained")
    rejects(.unauthorizedCandidate) {
      _ = try InputiaProfile.resolve(bundleIdentifier: "com.inputia.inputmethod.Inputia", info: [:], environment: ["INPUTIA_PROFILE_RUN_ID": "test"], applicationSupport: base)
    }
    rejects(.unauthorizedCandidate) {
      _ = try InputiaProfile.resolve(bundleIdentifier: candidateID, info: [:], environment: ["INPUTIA_PROFILE_RUN_ID": "test"], applicationSupport: base)
    }
    rejects(.unauthorizedCandidate) {
      _ = try InputiaProfile.resolve(bundleIdentifier: "com.inputia.inputmethod.Inputia", info: marked, environment: [:], applicationSupport: base)
    }
    rejects(.missingRunID) {
      _ = try InputiaProfile.resolve(bundleIdentifier: candidateID, info: marked, environment: [:], applicationSupport: base)
    }
    rejects(.unauthorizedCandidate) {
      _ = try InputiaProfile.resolve(bundleIdentifier: candidateID, info: ["InputiaDevelopmentCandidate": 1], environment: ["INPUTIA_PROFILE_RUN_ID": "test"], applicationSupport: base)
    }
    rejects(.invalidRunID) {
      _ = try InputiaProfile.resolve(bundleIdentifier: candidateID, info: ["InputiaDevelopmentCandidate": true, "InputiaProfileRunID": 1], environment: ["INPUTIA_PROFILE_RUN_ID": "test"], applicationSupport: base)
    }
    for value in ["", ".", "..", "../daily", "/tmp/profile", "a/b", "a\\b", "a.b", " ", "x\ny", "%2e%2e", "测试", String(repeating: "a", count: 65)] {
      rejects(.invalidRunID) {
        _ = try InputiaProfile.resolve(bundleIdentifier: candidateID, info: marked, environment: ["INPUTIA_PROFILE_RUN_ID": value], applicationSupport: base)
      }
    }
    let profile = try InputiaProfile.resolve(bundleIdentifier: candidateID, info: marked, environment: ["INPUTIA_PROFILE_RUN_ID": "run-20260905_A"], applicationSupport: base)
    let signedInfo: [String: Any] = ["InputiaDevelopmentCandidate": true, "InputiaProfileRunID": "run-20260905_A"]
    let plistProfile = try InputiaProfile.resolve(bundleIdentifier: candidateID, info: signedInfo, environment: [:], applicationSupport: base)
    check(plistProfile == profile, "launchd-safe signed metadata equals environment")
    rejects(.conflictingRunID) {
      _ = try InputiaProfile.resolve(bundleIdentifier: candidateID, info: signedInfo, environment: ["INPUTIA_PROFILE_RUN_ID": "other"], applicationSupport: base)
    }
    check(profile.root.path.hasSuffix("HandyUnifiedCandidate/run-20260905_A/Inputia"), "candidate fixed root")
    check(profile.handyRoot.path.hasSuffix("HandyUnifiedCandidate/run-20260905_A/Handy"), "paired Handy root")
    for path in profile.writablePaths {
      check(path.path.hasPrefix(profile.root.path + "/"), "all writes inside candidate root")
      check(!path.path.hasPrefix(daily.root.path + "/"), "no daily path leakage")
    }
    check(!profile.allowsHandyImport(daily.handyRoot.appendingPathComponent("history.db").path), "daily import rejected")
    check(!profile.allowsHandyImport(profile.handyRoot.appendingPathComponent("../Other/history.db").path), "unpaired import rejected")
    check(profile.allowsHandyImport(profile.handyRoot.appendingPathComponent("history.db").path), "paired import allowed")
    check(profile.allowsHandyImport(profile.handyRoot.appendingPathComponent("clipboard.db").path), "paired clipboard allowed")
    rejects(.pathOutsideProfile) { try profile.validateSettingsPath(daily.settings.path) }
    let migrated = try profile.isolatedSettings([
      "rime_user_data_dir": daily.rime.path, "memory_db_path": daily.memory.path,
      "candidate_page_size": 7,
    ], settingsPath: profile.settings.path)
    check(migrated["rime_user_data_dir"] as? String == profile.rime.path, "imported settings cannot select daily Rime")
    check(migrated["memory_db_path"] as? String == profile.memory.path, "imported settings cannot select daily memory")
    check(migrated["integration_outbox_path"] as? String == profile.outbox.path, "outbox path isolated")
    check(migrated["candidate_page_size"] as? Int == 7, "unrelated settings preserved")
    try profile.validateCandidatePaths()
    check(true, "missing candidate roots may be created after audit")
    try FileManager.default.createDirectory(at: profile.root, withIntermediateDirectories: true)
    try FileManager.default.createDirectory(at: daily.root, withIntermediateDirectories: true)
    try FileManager.default.createSymbolicLink(at: profile.rime, withDestinationURL: daily.root)
    rejects(.symbolicLink) { try profile.validateCandidatePaths() }
    try FileManager.default.removeItem(at: profile.rime)
    try FileManager.default.createDirectory(at: profile.handyRoot, withIntermediateDirectories: true)
    let history = profile.handyRoot.appendingPathComponent("history.db")
    try FileManager.default.createSymbolicLink(at: history, withDestinationURL: daily.memory)
    check(!profile.allowsHandyImport(history.path), "symlinked paired history rejected")
    try FileManager.default.removeItem(at: history)

    // 所有下方 daily 命名也仅指本次 Caches/UUID 下的合成目标，绝非日常数据。
    let original = Data("synthetic original must remain unchanged".utf8)
    try original.write(to: daily.memory)
    try FileManager.default.createDirectory(at: profile.rime, withIntermediateDirectories: true)
    let userdb = profile.rime.appendingPathComponent("luna_pinyin_simp.userdb", isDirectory: true)
    try FileManager.default.createSymbolicLink(at: userdb, withDestinationURL: daily.root)
    rejects(.symbolicLink) { try profile.validateCandidatePaths() }
    try FileManager.default.removeItem(at: userdb)
    try FileManager.default.createDirectory(at: userdb, withIntermediateDirectories: true)
    let userdbData = userdb.appendingPathComponent("000001.ldb")
    try FileManager.default.linkItem(at: daily.memory, to: userdbData)
    rejects(.hardLink) { try profile.validateCandidatePaths() }
    try FileManager.default.removeItem(at: userdbData)
    try FileManager.default.copyItem(at: daily.memory, to: userdbData)
    try profile.validateCandidatePaths()
    check(true, "independent copied userdb and normal nested directories accepted")

    for database in [profile.memory, history, profile.handyRoot.appendingPathComponent("clipboard.db")] {
      try FileManager.default.linkItem(at: daily.memory, to: database)
      rejects(.hardLink) { try profile.validateCandidatePaths() }
      if database != profile.memory {
        check(!profile.allowsHandyImport(database.path), "hardlinked paired DB import rejected")
      }
      try FileManager.default.removeItem(at: database)
      for suffix in ["-wal", "-shm", "-journal"] {
        let sidecar = URL(fileURLWithPath: database.path + suffix)
        try FileManager.default.createSymbolicLink(at: sidecar, withDestinationURL: daily.memory)
        rejects(.symbolicLink) { try profile.validateCandidatePaths() }
        rejects(.symbolicLink) { try profile.validateSettingsPath(profile.settings.path) }
        if database != profile.memory {
          check(!profile.allowsHandyImport(database.path), "linked sidecar blocks paired DB import")
        }
        try FileManager.default.removeItem(at: sidecar)
      }
    }
    let hidden = profile.handyRoot.appendingPathComponent(".hidden", isDirectory: true)
    try FileManager.default.createDirectory(at: hidden, withIntermediateDirectories: true)
    let hiddenLink = hidden.appendingPathComponent("broken-link")
    try FileManager.default.createSymbolicLink(at: hiddenLink, withDestinationURL: daily.root.appendingPathComponent("nonexistent"))
    rejects(.symbolicLink) { try profile.validateCandidatePaths() }
    try FileManager.default.removeItem(at: hiddenLink)
    try profile.validateCandidatePaths()
    check(true, "normal hidden directory accepted and no stale rejection after repairs")
    check(try Data(contentsOf: daily.memory) == original, "synthetic linked target remained untouched")
    print("unifiedInputProfileSelfCheck=true checks=\(checks)")
  }
}
