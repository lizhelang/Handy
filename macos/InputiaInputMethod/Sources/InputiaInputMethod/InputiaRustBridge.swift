import Foundation

private let keyBackspace: Int32 = 1
private let keyEscape: Int32 = 2
private let keySpace: Int32 = 3
private let keyShift: Int32 = 4
private let keyPageDown: Int32 = 5
private let keyPageUp: Int32 = 6
private let keyEnter: Int32 = 7
private let keyTogglePunctuation: Int32 = 8
private let keyToggleCharacterWidth: Int32 = 9
private let keyToggleInputMode: Int32 = 10
private let inputModeEnglish: Int32 = 1
private let inputModeChinese: Int32 = 2
private let sourceTyped: Int32 = 1
private let sourceClipboard: Int32 = 3
private let defaultCandidatePageSize = 7

@_silgen_name("inputia_session_new_luna_pinyin_simp")
private func inputia_session_new_luna_pinyin_simp(
  _ userDataDir: UnsafePointer<CChar>,
  _ candidatePageSize: Int
) -> UnsafeMutableRawPointer?

@_silgen_name("inputia_session_new_luna_pinyin_simp_with_memory")
private func inputia_session_new_luna_pinyin_simp_with_memory(
  _ userDataDir: UnsafePointer<CChar>,
  _ memoryDbPath: UnsafePointer<CChar>,
  _ candidatePageSize: Int
) -> UnsafeMutableRawPointer?

@_silgen_name("inputia_session_new_from_settings")
private func inputia_session_new_from_settings(
  _ settingsPath: UnsafePointer<CChar>
) -> UnsafeMutableRawPointer?

@_silgen_name("inputia_session_new_from_settings_without_memory")
private func inputia_session_new_from_settings_without_memory(
  _ settingsPath: UnsafePointer<CChar>
) -> UnsafeMutableRawPointer?

@_silgen_name("inputia_session_free")
private func inputia_session_free(_ session: UnsafeMutableRawPointer?)

@_silgen_name("inputia_session_handle_char")
private func inputia_session_handle_char(
  _ session: UnsafeMutableRawPointer?,
  _ unicodeScalar: UInt32
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_handle_digit")
private func inputia_session_handle_digit(
  _ session: UnsafeMutableRawPointer?,
  _ digit: UInt8
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_handle_special")
private func inputia_session_handle_special(
  _ session: UnsafeMutableRawPointer?,
  _ specialKey: Int32
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_set_input_mode")
private func inputia_session_set_input_mode(
  _ session: UnsafeMutableRawPointer?,
  _ inputMode: Int32
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_learn")
private func inputia_session_learn(
  _ session: UnsafeMutableRawPointer?,
  _ source: Int32,
  _ text: UnsafePointer<CChar>,
  _ bundleId: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_import_handy_history")
private func inputia_session_import_handy_history(
  _ session: UnsafeMutableRawPointer?,
  _ historyDbPath: UnsafePointer<CChar>,
  _ bundleId: UnsafePointer<CChar>,
  _ limit: Int
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_import_handy_clipboard")
private func inputia_session_import_handy_clipboard(
  _ session: UnsafeMutableRawPointer?,
  _ clipboardDbPath: UnsafePointer<CChar>,
  _ bundleId: UnsafePointer<CChar>,
  _ limit: Int
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_voice_hotwords")
private func inputia_session_voice_hotwords(
  _ session: UnsafeMutableRawPointer?,
  _ limit: Int
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_clipboard_candidates")
private func inputia_session_clipboard_candidates(
  _ session: UnsafeMutableRawPointer?,
  _ limit: Int
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_completion_candidates")
private func inputia_session_completion_candidates(
  _ session: UnsafeMutableRawPointer?,
  _ prefix: UnsafePointer<CChar>,
  _ limit: Int
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_set_app_context")
private func inputia_session_set_app_context(
  _ session: UnsafeMutableRawPointer?,
  _ bundleId: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_set_app_context_with_window")
private func inputia_session_set_app_context_with_window(
  _ session: UnsafeMutableRawPointer?,
  _ bundleId: UnsafePointer<CChar>,
  _ windowTitle: UnsafePointer<CChar>
) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_string_free")
private func inputia_string_free(_ value: UnsafeMutablePointer<CChar>?)

struct InputiaBridgeOutcome {
  let ok: Bool
  let consumed: Bool
  let commit: String?
  let mode: String
  let composing: String
  let page: Int
  let candidates: [String]

  static let error = InputiaBridgeOutcome(
    ok: false,
    consumed: false,
    commit: nil,
    mode: "English",
    composing: "",
    page: 0,
    candidates: []
  )

  init(
    ok: Bool,
    consumed: Bool,
    commit: String?,
    mode: String,
    composing: String,
    page: Int,
    candidates: [String]
  ) {
    self.ok = ok
    self.consumed = consumed
    self.commit = commit
    self.mode = mode
    self.composing = composing
    self.page = page
    self.candidates = candidates
  }

  init(dictionary: [String: Any]) {
    ok = dictionary["ok"] as? Bool ?? false
    consumed = dictionary["consumed"] as? Bool ?? false
    commit = dictionary["commit"] as? String
    mode = dictionary["mode"] as? String ?? "English"
    composing = dictionary["composing"] as? String ?? ""
    page = dictionary["page"] as? Int ?? 0
    let rawCandidates = dictionary["visible_candidates"] as? [[String: Any]] ?? []
    candidates = rawCandidates.compactMap { $0["text"] as? String }
  }
}

final class InputiaRustBridge {
  static let shared = InputiaRustBridge(settingsPath: defaultSettingsPath())

  private let settingsPath: String
  private var session: UnsafeMutableRawPointer?
  private var settingsModificationDate: Date?
  private var cachedInputModeToggleShortcut = "shift"
  private var cachedScriptToggleShortcut = "control_shift_s"
  private(set) var latestOutcome = InputiaBridgeOutcome.error

  private init(settingsPath: String, startInChineseMode: Bool = false) {
    self.settingsPath = settingsPath
    Self.ensureSettingsFile(at: settingsPath)
    settingsModificationDate = Self.modificationDate(for: settingsPath)
    cachedInputModeToggleShortcut = Self.inputModeToggleShortcut(in: settingsPath)
    cachedScriptToggleShortcut = Self.scriptToggleShortcut(in: settingsPath)
    session = Self.openSettingsSession(settingsPath: settingsPath)
    if startInChineseMode {
      _ = setChineseMode()
    }
  }

  static func makeDefault() -> InputiaRustBridge {
    InputiaRustBridge(settingsPath: defaultSettingsPath(), startInChineseMode: true)
  }

  private init(userDataDir: String, memoryDbPath: String?) {
    settingsPath = Self.defaultSettingsPath()
    session = Self.openDirectSession(userDataDir: userDataDir, memoryDbPath: memoryDbPath)
    if session == nil {
      NSLog("Inputia Rust bridge failed to initialize session")
    }
  }

  static func temporaryDirectForDiagnostics() -> InputiaRustBridge {
    let root = URL(fileURLWithPath: NSTemporaryDirectory())
      .appendingPathComponent("InputiaDirectBridgeSelfCheck-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
    return InputiaRustBridge(
      userDataDir: root.appendingPathComponent("rime", isDirectory: true).path,
      memoryDbPath: nil
    )
  }

  static func temporaryForDiagnostics() -> InputiaRustBridge {
    let root = URL(fileURLWithPath: NSTemporaryDirectory())
      .appendingPathComponent("InputiaBridgeSelfCheck-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
    return InputiaRustBridge(
      userDataDir: root.appendingPathComponent("rime", isDirectory: true).path,
      memoryDbPath: root.appendingPathComponent("inputia_memory.db").path
    )
  }

  static func temporarySettingsForDiagnostics() -> InputiaRustBridge {
    let root = URL(fileURLWithPath: NSTemporaryDirectory())
      .appendingPathComponent("InputiaSettingsSelfCheck-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
    return InputiaRustBridge(settingsPath: root.appendingPathComponent("settings.json").path)
  }

  deinit {
    inputia_session_free(session)
  }

  func handle(character: Character) -> InputiaBridgeOutcome {
    guard let scalar = character.unicodeScalars.first else {
      return latestOutcome
    }
    if character.isNumber, let digit = UInt8(String(character)) {
      return consume(inputia_session_handle_digit(session, digit))
    }
    return consume(inputia_session_handle_char(session, scalar.value))
  }

  func chooseCandidate(atZeroBasedIndex index: Int) -> InputiaBridgeOutcome {
    guard index >= 0, index < 9 else {
      return latestOutcome
    }
    return consume(inputia_session_handle_digit(session, UInt8(index + 1)))
  }

  func handleSpecial(_ specialKey: Int32) -> InputiaBridgeOutcome {
    consume(inputia_session_handle_special(session, specialKey))
  }

  func toggleInputMode() -> InputiaBridgeOutcome {
    handleSpecial(keyToggleInputMode)
  }

  func togglePunctuationPreference() -> InputiaBridgeOutcome {
    handleSpecial(keyTogglePunctuation)
  }

  func toggleCharacterWidthPreference() -> InputiaBridgeOutcome {
    handleSpecial(keyToggleCharacterWidth)
  }

  func setChineseMode() -> InputiaBridgeOutcome {
    consume(inputia_session_set_input_mode(session, inputModeChinese))
  }

  @discardableResult
  func reloadSettingsIfNeeded() -> Bool {
    let currentModificationDate = Self.modificationDate(for: settingsPath)
    guard currentModificationDate != settingsModificationDate else {
      return false
    }
    return reloadSettings(newModificationDate: currentModificationDate)
  }

  func inputModeToggleShortcut() -> String {
    cachedInputModeToggleShortcut
  }

  func scriptToggleShortcut() -> String {
    cachedScriptToggleShortcut
  }

  @discardableResult
  func toggleChineseScriptPreference() -> Bool {
    guard Self.toggleChineseScript(in: settingsPath) else {
      return false
    }
    return reloadSettings(newModificationDate: Self.modificationDate(for: settingsPath))
  }

  func backspace() -> InputiaBridgeOutcome {
    handleSpecial(keyBackspace)
  }

  func escape() -> InputiaBridgeOutcome {
    handleSpecial(keyEscape)
  }

  func space() -> InputiaBridgeOutcome {
    handleSpecial(keySpace)
  }

  func enter() -> InputiaBridgeOutcome {
    handleSpecial(keyEnter)
  }

  func pageDown() -> InputiaBridgeOutcome {
    handleSpecial(keyPageDown)
  }

  func pageUp() -> InputiaBridgeOutcome {
    handleSpecial(keyPageUp)
  }

  func setAppContext(bundleId: String, windowTitle: String? = nil) -> Bool {
    guard let session else {
      return false
    }
    let raw: UnsafeMutablePointer<CChar>? = bundleId.withCString { bundlePointer in
      if let windowTitle, !windowTitle.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
        return windowTitle.withCString { windowPointer in
          inputia_session_set_app_context_with_window(session, bundlePointer, windowPointer)
        }
      }
      return inputia_session_set_app_context(session, bundlePointer)
    }
    guard let raw else {
      return false
    }
    defer { inputia_string_free(raw) }
    guard
      let dictionary = parseJson(raw),
      let ok = dictionary["ok"] as? Bool
    else {
      return false
    }
    return ok
  }

  static func debugSettingsReloadSelfCheck(settingsPath: String) -> [InputiaBridgeOutcome] {
    let bridge = InputiaRustBridge(settingsPath: settingsPath, startInChineseMode: true)
    var outcomes: [InputiaBridgeOutcome] = []
    outcomes.append(bridge.toggleInputMode())
    Self.writeSettings(
      shiftToggleEnabled: false,
      inputModeToggleShortcut: "none",
      punctuationPreference: "english_in_chinese",
      candidatePageSize: 7,
      to: settingsPath
    )
    bridge.reloadSettingsIfNeeded()
    outcomes.append(bridge.handleSpecial(keyShift))
    return outcomes
  }

  static func debugCandidateCountFallbackSelfCheck(settingsPath: String) -> InputiaBridgeOutcome {
    writeSettings(
      shiftToggleEnabled: true,
      punctuationPreference: "english_in_chinese",
      candidatePageSize: 8,
      to: settingsPath
    )
    let settingsURL = URL(fileURLWithPath: settingsPath)
    if
      let data = try? Data(contentsOf: settingsURL),
      let object = try? JSONSerialization.jsonObject(with: data),
      var dictionary = object as? [String: Any]
    {
      dictionary["schema_id"] = "double_pinyin"
      let invalidMemoryURL = settingsURL.deletingLastPathComponent()
        .appendingPathComponent("memory-as-directory", isDirectory: true)
      try? FileManager.default.createDirectory(at: invalidMemoryURL, withIntermediateDirectories: true)
      dictionary["memory_db_path"] = invalidMemoryURL.path
      if let updated = try? JSONSerialization.data(
        withJSONObject: dictionary,
        options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
      ) {
        try? updated.write(to: settingsURL, options: .atomic)
      }
    }

    let bridge = InputiaRustBridge(settingsPath: settingsPath, startInChineseMode: true)
    var outcome = bridge.latestOutcome
    for character in "yh" {
      outcome = bridge.handle(character: character)
    }
    return outcome
  }

  static func debugClipboardPrivacySelfCheck(settingsPath: String) -> [String: Bool] {
    Self.writeSettings(
      shiftToggleEnabled: true,
      punctuationPreference: "english_in_chinese",
      candidatePageSize: 7,
      to: settingsPath
    )
    let bridge = InputiaRustBridge(settingsPath: settingsPath)
    return [
      "textedit": bridge.shouldReadClipboard(bundleId: "com.apple.TextEdit"),
      "onepassword": bridge.shouldReadClipboard(bundleId: "com.1password.1password"),
      "unknown": bridge.shouldReadClipboard(bundleId: "unknown"),
      "privateWindow": bridge.shouldReadClipboard(
        bundleId: "com.apple.Safari",
        windowTitle: "Private Browsing - Bank Login"
      ),
    ]
  }

  func debugCandidatePageSizeSelfCheck() -> InputiaBridgeOutcome {
    _ = setChineseMode()
    var outcome = latestOutcome
    for character in "ni" {
      outcome = handle(character: character)
    }
    return outcome
  }

  func debugFullPinyinSelfCheck() -> [InputiaBridgeOutcome] {
    var outcomes: [InputiaBridgeOutcome] = []
    outcomes.append(toggleInputMode())
    for character in "zhongguo" {
      outcomes.append(handle(character: character))
    }
    outcomes.append(space())
    return outcomes
  }

  func debugDefaultChineseSelfCheck() -> [InputiaBridgeOutcome] {
    var outcomes: [InputiaBridgeOutcome] = []
    for character in "ni" {
      outcomes.append(handle(character: character))
    }
    outcomes.append(space())
    return outcomes
  }

  func debugMemorySelfCheck() -> [InputiaBridgeOutcome] {
    _ = learnClipboard(text: "种过", bundleId: "com.apple.TextEdit")
    return debugFullPinyinSelfCheck()
  }

  func debugClipboardRecallSelfCheck() -> [String] {
    _ = learnClipboard(text: "剪贴板 常用语", bundleId: "com.apple.TextEdit")
    _ = learnClipboard(text: "剪贴板 临时句", bundleId: "com.apple.TextEdit")
    return clipboardCandidates(limit: 5)
  }

  func debugEnglishCompletionSelfCheck() -> [String] {
    _ = learnTyped(text: "Inputia", bundleId: "com.apple.TextEdit")
    _ = learnTyped(text: "Inputia", bundleId: "com.apple.TextEdit")
    _ = learnClipboard(text: "input-layer", bundleId: "com.apple.TextEdit")
    return completionCandidates(prefix: "in", limit: 5)
  }

  func debugSettingsSelfCheck() -> [InputiaBridgeOutcome] {
    debugFullPinyinSelfCheck()
  }

  func shouldReadClipboard(bundleId: String, windowTitle: String? = nil) -> Bool {
    guard !bundleId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, bundleId != "unknown" else {
      return false
    }
    guard !isSensitiveApp(bundleId: bundleId, windowTitle: windowTitle) else {
      return false
    }
    let settings = Self.loadSettingsDictionary(path: settingsPath) ?? [:]
    let memoryEnabled = settings["memory_enabled"] as? Bool ?? true
    let privacyLearningEnabled = settings["privacy_learning_enabled"] as? Bool ?? true
    return memoryEnabled && privacyLearningEnabled
  }

  func isSensitiveApp(bundleId: String, windowTitle: String? = nil) -> Bool {
    guard !bundleId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, bundleId != "unknown" else {
      return true
    }
    Self.ensureSettingsFile(at: settingsPath)
    let settings = Self.loadSettingsDictionary(path: settingsPath) ?? [:]
    let sensitiveBundleIds = (settings["sensitive_bundle_ids"] as? [String]).flatMap { values in
      values.isEmpty ? nil : values
    } ?? Self.defaultSensitiveBundleIds
    if sensitiveBundleIds.contains(bundleId) {
      return true
    }
    return Self.isSensitiveWindowTitle(windowTitle)
  }

  func learnClipboard(text: String, bundleId: String, windowTitle: String? = nil) -> Bool {
    learn(source: sourceClipboard, text: text, bundleId: bundleId, windowTitle: windowTitle)
  }

  func learnTyped(text: String, bundleId: String, windowTitle: String? = nil) -> Bool {
    learn(source: sourceTyped, text: text, bundleId: bundleId, windowTitle: windowTitle)
  }

  private func learn(source: Int32, text: String, bundleId: String, windowTitle: String? = nil) -> Bool {
    guard let session else {
      return false
    }
    _ = setAppContext(bundleId: bundleId, windowTitle: windowTitle)
    let raw = text.withCString { textPointer in
      bundleId.withCString { bundlePointer in
        inputia_session_learn(session, source, textPointer, bundlePointer)
      }
    }
    guard let raw else {
      return false
    }
    defer { inputia_string_free(raw) }
    guard
      let dictionary = parseJson(raw),
      let ok = dictionary["ok"] as? Bool
    else {
      return false
    }
    return ok
  }

  func importHandyHistory(path: String, bundleId: String, limit: Int) -> Int? {
    guard let session else {
      return nil
    }
    let raw = path.withCString { pathPointer in
      bundleId.withCString { bundlePointer in
        inputia_session_import_handy_history(session, pathPointer, bundlePointer, limit)
      }
    }
    return importedCount(from: raw)
  }

  func importHandyClipboard(path: String, bundleId: String, limit: Int) -> Int? {
    guard let session else {
      return nil
    }
    let raw = path.withCString { pathPointer in
      bundleId.withCString { bundlePointer in
        inputia_session_import_handy_clipboard(session, pathPointer, bundlePointer, limit)
      }
    }
    return importedCount(from: raw)
  }

  func voiceHotwords(limit: Int) -> [String] {
    guard let session else {
      return []
    }
    let raw = inputia_session_voice_hotwords(session, limit)
    guard let raw else {
      return []
    }
    defer { inputia_string_free(raw) }
    guard
      let dictionary = parseJson(raw),
      let ok = dictionary["ok"] as? Bool,
      ok,
      let hotwords = dictionary["hotwords"] as? [String]
    else {
      return []
    }
    return hotwords
  }

  func clipboardCandidates(limit: Int) -> [String] {
    guard let session else {
      return []
    }
    let raw = inputia_session_clipboard_candidates(session, limit)
    guard let raw else {
      return []
    }
    defer { inputia_string_free(raw) }
    guard
      let dictionary = parseJson(raw),
      let ok = dictionary["ok"] as? Bool,
      ok,
      let rawCandidates = dictionary["candidates"] as? [[String: Any]]
    else {
      return []
    }
    return rawCandidates.compactMap { $0["text"] as? String }
  }

  func completionCandidates(prefix: String, limit: Int) -> [String] {
    guard let session else {
      return []
    }
    let raw = prefix.withCString { prefixPointer in
      inputia_session_completion_candidates(session, prefixPointer, limit)
    }
    guard let raw else {
      return []
    }
    defer { inputia_string_free(raw) }
    guard
      let dictionary = parseJson(raw),
      let ok = dictionary["ok"] as? Bool,
      ok,
      let rawCandidates = dictionary["candidates"] as? [[String: Any]]
    else {
      return []
    }
    return rawCandidates.compactMap { $0["text"] as? String }
  }

  private func importedCount(from raw: UnsafeMutablePointer<CChar>?) -> Int? {
    guard let raw else {
      return nil
    }
    defer { inputia_string_free(raw) }
    guard
      let dictionary = parseJson(raw),
      let ok = dictionary["ok"] as? Bool,
      ok
    else {
      return nil
    }
    return dictionary["imported"] as? Int
  }

  private func reloadSettings(newModificationDate: Date?) -> Bool {
    let previousMode = latestOutcome.mode
    let oldSession = session
    session = nil
    inputia_session_free(oldSession)

    guard let newSession = Self.openSettingsSession(settingsPath: settingsPath) else {
      NSLog("Inputia Rust bridge failed to reload settings session")
      latestOutcome = .error
      return false
    }

    session = newSession
    settingsModificationDate = newModificationDate
    cachedInputModeToggleShortcut = Self.inputModeToggleShortcut(in: settingsPath)
    cachedScriptToggleShortcut = Self.scriptToggleShortcut(in: settingsPath)

    switch previousMode {
    case "Chinese":
      _ = consume(inputia_session_set_input_mode(session, inputModeChinese))
    case "English":
      _ = consume(inputia_session_set_input_mode(session, inputModeEnglish))
    default:
      latestOutcome = InputiaBridgeOutcome.error
    }
    return true
  }

  private func consume(_ raw: UnsafeMutablePointer<CChar>?) -> InputiaBridgeOutcome {
    guard let raw else {
      latestOutcome = .error
      return latestOutcome
    }
    defer { inputia_string_free(raw) }

    let json = String(cString: raw)
    guard
      let dictionary = Self.parseJsonString(json)
    else {
      latestOutcome = .error
      return latestOutcome
    }

    latestOutcome = InputiaBridgeOutcome(dictionary: dictionary)
    return latestOutcome
  }

  private static func defaultUserDataDir() -> String {
    let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
      .first
      ?? URL(fileURLWithPath: NSTemporaryDirectory())
    let dir = base.appendingPathComponent("Inputia/rime", isDirectory: true)
    return dir.path
  }

  private static func defaultMemoryDbPath() -> String {
    let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
      .first
      ?? URL(fileURLWithPath: NSTemporaryDirectory())
    return base.appendingPathComponent("Inputia/inputia_memory.db").path
  }

  private static func defaultSettingsPath() -> String {
    let base = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)
      .first
      ?? URL(fileURLWithPath: NSTemporaryDirectory())
    return base.appendingPathComponent("Inputia/settings.json").path
  }

  private static func modificationDate(for path: String) -> Date? {
    guard
      let attributes = try? FileManager.default.attributesOfItem(atPath: path),
      let date = attributes[.modificationDate] as? Date
    else {
      return nil
    }
    return date
  }

  private static func openSettingsSession(settingsPath: String) -> UnsafeMutableRawPointer? {
    let session = settingsPath.withCString { pointer in
      inputia_session_new_from_settings(pointer)
    }
    if session == nil {
      NSLog("Inputia Rust bridge failed to initialize settings session; retrying without memory")
      let fallback = settingsPath.withCString { pointer in
        inputia_session_new_from_settings_without_memory(pointer)
      }
      if fallback == nil {
        NSLog("Inputia Rust bridge failed to initialize settings session without memory")
      }
      return fallback
    }
    return session
  }

  private static func ensureSettingsFile(at path: String) {
    let url = URL(fileURLWithPath: path)
    if FileManager.default.fileExists(atPath: path) {
      patchSettingsFileIfNeeded(url: url)
      return
    }
    writeSettings(
      shiftToggleEnabled: true,
      punctuationPreference: "english_in_chinese",
      candidatePageSize: 7,
      to: path
    )
  }

  private static func patchSettingsFileIfNeeded(url: URL) {
    guard
      let data = try? Data(contentsOf: url),
      let object = try? JSONSerialization.jsonObject(with: data),
      var dictionary = object as? [String: Any]
    else {
      return
    }

    var changed = false
    let existingSharedDataDir = dictionary["rime_shared_data_dir"] as? String
    if existingSharedDataDir == nil
      || !FileManager.default.fileExists(atPath: existingSharedDataDir ?? ""),
      let bundledRimeDataPath
    {
      dictionary["rime_shared_data_dir"] = bundledRimeDataPath
      changed = true
    }
    if dictionary["rime_user_data_dir"] == nil {
      dictionary["rime_user_data_dir"] = url.deletingLastPathComponent().appendingPathComponent("rime").path
      changed = true
    }
    if dictionary["memory_db_path"] == nil {
      dictionary["memory_db_path"] = url.deletingLastPathComponent().appendingPathComponent("inputia_memory.db").path
      changed = true
    }
    if dictionary["character_width_preference"] == nil {
      dictionary["character_width_preference"] = "half_width"
      changed = true
    }
    if dictionary["spelling_correction_enabled"] == nil {
      dictionary["spelling_correction_enabled"] = true
      changed = true
    }
    if dictionary["input_mode_toggle_shortcut"] == nil {
      let shiftToggleEnabled = dictionary["shift_toggle_enabled"] as? Bool ?? true
      dictionary["input_mode_toggle_shortcut"] = shiftToggleEnabled ? "shift" : "none"
      changed = true
    }
    if dictionary["chinese_script"] == nil {
      dictionary["chinese_script"] = "simplified"
      changed = true
    }
    if dictionary["script_toggle_shortcut"] == nil {
      dictionary["script_toggle_shortcut"] = "control_shift_s"
      changed = true
    }
    var sensitiveBundleIds = (dictionary["sensitive_bundle_ids"] as? [String]) ?? []
    for bundleId in Self.defaultSensitiveBundleIds where !sensitiveBundleIds.contains(bundleId) {
      sensitiveBundleIds.append(bundleId)
      changed = true
    }
    dictionary["sensitive_bundle_ids"] = sensitiveBundleIds

    guard changed, let patched = try? JSONSerialization.data(
      withJSONObject: dictionary,
      options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
    ) else {
      return
    }
    try? patched.write(to: url, options: .atomic)
  }

  private static func loadSettingsDictionary(path: String) -> [String: Any]? {
    let url = URL(fileURLWithPath: path)
    guard
      let data = try? Data(contentsOf: url),
      let object = try? JSONSerialization.jsonObject(with: data),
      let dictionary = object as? [String: Any]
    else {
      return nil
    }
    return dictionary
  }

  private static func inputModeToggleShortcut(in path: String) -> String {
    guard let dictionary = loadSettingsDictionary(path: path) else {
      return "shift"
    }
    if
      let value = dictionary["input_mode_toggle_shortcut"] as? String,
      ["shift", "control_space", "none"].contains(value)
    {
      return value
    }
    let shiftToggleEnabled = dictionary["shift_toggle_enabled"] as? Bool ?? true
    return shiftToggleEnabled ? "shift" : "none"
  }

  private static func scriptToggleShortcut(in path: String) -> String {
    guard let dictionary = loadSettingsDictionary(path: path) else {
      return "control_shift_s"
    }
    if
      let value = dictionary["script_toggle_shortcut"] as? String,
      ["control_shift_s", "none"].contains(value)
    {
      return value
    }
    return "control_shift_s"
  }

  private static func toggleChineseScript(in path: String) -> Bool {
    let url = URL(fileURLWithPath: path)
    guard var dictionary = loadSettingsDictionary(path: path) else {
      return false
    }
    let current = dictionary["chinese_script"] as? String
    dictionary["chinese_script"] = current == "traditional" ? "simplified" : "traditional"
    guard let data = try? JSONSerialization.data(
      withJSONObject: dictionary,
      options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
    ) else {
      return false
    }
    do {
      try data.write(to: url, options: .atomic)
      return true
    } catch {
      NSLog("Inputia failed to toggle Chinese script: \(error)")
      return false
    }
  }

  private static func isSensitiveWindowTitle(_ windowTitle: String?) -> Bool {
    guard let windowTitle else {
      return false
    }
    let value = windowTitle.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
    guard !value.isEmpty else {
      return false
    }
    return [
      "private browsing",
      "private window",
      "incognito",
      "隐私浏览",
      "无痕",
      "登录",
      "登陆",
      "登入",
      "密码",
      "账号",
      "账户",
      "验证码",
      "身份验证",
      "认证",
      "password",
      "login",
      "log in",
      "sign in",
      "signin",
      "sign-in",
      "authentication",
      "otp",
      "2fa",
      "银行",
      "bank",
      "医疗",
      "medical",
    ].contains { value.contains($0) }
  }

  private static func openDirectSession(userDataDir: String, memoryDbPath: String?) -> UnsafeMutableRawPointer? {
    if let memoryDbPath {
      return userDataDir.withCString { userPointer in
        memoryDbPath.withCString { memoryPointer in
          inputia_session_new_luna_pinyin_simp_with_memory(
            userPointer,
            memoryPointer,
            defaultCandidatePageSize
          )
        }
      }
    }
    return userDataDir.withCString { pointer in
      inputia_session_new_luna_pinyin_simp(pointer, defaultCandidatePageSize)
    }
  }

  private func parseJson(_ raw: UnsafeMutablePointer<CChar>) -> [String: Any]? {
    Self.parseJsonString(String(cString: raw))
  }

  private static func parseJsonString(_ json: String) -> [String: Any]? {
    guard
      let data = json.data(using: .utf8),
      let object = try? JSONSerialization.jsonObject(with: data),
      let dictionary = object as? [String: Any]
    else {
      return nil
    }
    return dictionary
  }

  private static func writeSettings(
    shiftToggleEnabled: Bool,
    inputModeToggleShortcut: String? = nil,
    punctuationPreference: String,
    candidatePageSize: Int,
    characterWidthPreference: String = "half_width",
    chineseScript: String = "simplified",
    to path: String
  ) {
    let url = URL(fileURLWithPath: path)
    try? FileManager.default.createDirectory(
      at: url.deletingLastPathComponent(),
      withIntermediateDirectories: true
    )
    let shortcut = inputModeToggleShortcut ?? (shiftToggleEnabled ? "shift" : "none")
    var dictionary: [String: Any] = [
      "candidate_font_size": 14,
      "candidate_page_size": candidatePageSize,
      "character_width_preference": characterWidthPreference,
      "chinese_script": chineseScript,
      "input_mode_toggle_shortcut": shortcut,
      "memory_enabled": true,
      "memory_db_path": url.deletingLastPathComponent().appendingPathComponent("inputia_memory.db").path,
      "privacy_learning_enabled": true,
      "punctuation_preference": punctuationPreference,
      "rime_user_data_dir": url.deletingLastPathComponent().appendingPathComponent("rime").path,
      "schema_id": "luna_pinyin_simp",
      "script_toggle_shortcut": "control_shift_s",
      "sensitive_bundle_ids": Self.defaultSensitiveBundleIds,
      "shift_toggle_enabled": shortcut == "shift",
      "spelling_correction_enabled": true,
    ]
    if let bundledRimeDataPath {
      dictionary["rime_shared_data_dir"] = bundledRimeDataPath
    }
    let data = try? JSONSerialization.data(
      withJSONObject: dictionary,
      options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
    )
    try? data?.write(to: url, options: .atomic)
  }

  private static var bundledRimeDataPath: String? {
    let candidates = [
      Bundle.main.resourceURL?.appendingPathComponent("RimeData", isDirectory: true),
      URL(fileURLWithPath: "/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/RimeData", isDirectory: true),
    ].compactMap { $0 }

    for url in candidates where FileManager.default.fileExists(atPath: url.path) {
      return url.path
    }
    return nil
  }

  private static let defaultSensitiveBundleIds = [
    "com.1password.1password",
    "com.agilebits.onepassword7",
    "com.apple.Safari.PrivateBrowsing",
    "com.apple.SecurityAgent",
    "com.bitwarden.desktop",
    "com.lastpass.LastPass",
    "com.protonmail.protonmail",
  ]
}
