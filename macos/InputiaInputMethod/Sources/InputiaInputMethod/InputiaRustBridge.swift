import Foundation

@_silgen_name("inputia_session_set_context_unverified")
private func inputia_session_set_context_unverified(_ session: UnsafeMutableRawPointer?, _ bundleId: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

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

#if INPUTIA_PAIRED_BUILD
@_silgen_name("inputia_session_undo_recent_learning")
private func inputia_session_undo_recent_learning(_ session: UnsafeMutableRawPointer?) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_candidate_pool")
private func inputia_session_candidate_pool(_ session: UnsafeMutableRawPointer?, _ limit: Int) -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_session_choose_candidate_id")
private func inputia_session_choose_candidate_id(_ session: UnsafeMutableRawPointer?, _ composing: UnsafePointer<CChar>, _ id: UnsafePointer<CChar>, _ expectedText: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?

@_silgen_name("inputia_session_shared_candidate_order")
private func inputia_session_shared_candidate_order(_ session: UnsafeMutableRawPointer?, _ terms: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?
#endif

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
  let candidateIDs: [String]

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
    candidates: [String],
    candidateIDs: [String] = []
  ) {
    self.ok = ok
    self.consumed = consumed
    self.commit = commit
    self.mode = mode
    self.composing = composing
    self.page = page
    self.candidates = candidates
    self.candidateIDs = candidateIDs
  }

  /// stale ID/预期文本校验失败时，保留当前组合与映射，不能接受错误包的空composing。
  static func failedSelection(preserving previous: InputiaBridgeOutcome) -> InputiaBridgeOutcome {
    InputiaBridgeOutcome(ok: false, consumed: true, commit: nil, mode: previous.mode,
      composing: previous.composing, page: previous.page, candidates: previous.candidates,
      candidateIDs: previous.candidateIDs)
  }

  static func decodedSelection(_ dictionary: [String: Any]?, preserving previous: InputiaBridgeOutcome) -> InputiaBridgeOutcome {
    guard let dictionary else { return .failedSelection(preserving: previous) }
    let result = InputiaBridgeOutcome(dictionary: dictionary)
    return result.ok ? result : .failedSelection(preserving: previous)
  }

  init(dictionary: [String: Any]) {
    ok = dictionary["ok"] as? Bool ?? false
    consumed = dictionary["consumed"] as? Bool ?? false
    commit = dictionary["commit"] as? String
    mode = dictionary["mode"] as? String ?? "English"
    composing = dictionary["composing"] as? String ?? ""
    page = dictionary["page"] as? Int ?? 0
    let rawCandidates = dictionary["visible_candidates"] as? [[String: Any]] ?? []
    let visible = rawCandidates.filter { $0["text"] is String }
    candidates = visible.compactMap { $0["text"] as? String }
    candidateIDs = visible.map { $0["id"] as? String ?? "" }
  }
}

final class InputiaRustBridge {
  static let shared = InputiaRustBridge(settingsPath: defaultSettingsPath())

  private let settingsPath: String
  private var session: UnsafeMutableRawPointer?
  private var settingsCache: InputiaSettingsCache?
  private var activeSettings: InputiaSettingsStore.Snapshot?
  private let settingsRetry = InputiaSettingsRetryGate()
  private var diagnosticSettings = false
  private var scriptEdit: InputiaSettingsEdit?
  private var candidateDisplayIdentity: String?
  struct SettingsApplication {
    let snapshot: InputiaSettingsStore.Snapshot
    let sessionOpened: Bool
    let withoutMemory: Bool
    let nativeFields: [String]
    let failureCode: String?
  }
  private(set) var settingsApplication: SettingsApplication?
  var settingsApplicationDidChange: ((SettingsApplication) -> Void)?
  private var cachedInputModeToggleShortcut = "shift"
  private var cachedScriptToggleShortcut = "control_shift_s"
  private(set) var schemaID = "luna_pinyin_simp"
  var usesNaturalDoublePinyin: Bool { schemaID == "double_pinyin" }
  private(set) var latestOutcome = InputiaBridgeOutcome.error

  private init(settingsPath: String, startInChineseMode: Bool = false, diagnostics: Bool = false) {
    self.settingsPath = settingsPath
    diagnosticSettings = diagnostics
    do {
      if diagnostics { try Self.validateDiagnosticSettingsPath(settingsPath) }
      else { try InputiaProfile.current.validateCandidatePaths(); try InputiaProfile.current.validateSettingsPath(settingsPath) }
      let cache = InputiaSettingsCache.shared(path: settingsPath)
      settingsCache = cache
      let state = cache.state
      if let snapshot = state.snapshot, settingsRetry.begin(identity: snapshot.identity, generation: state.generation) { _ = reloadSettings(snapshot: snapshot) }
    } catch { NSLog("Inputia settings startup rejected") }
    if startInChineseMode { _ = setChineseMode() }
  }

  static func makeDefault() -> InputiaRustBridge {
    InputiaRustBridge(settingsPath: defaultSettingsPath(), startInChineseMode: true)
  }

  private init(directDiagnosticWithMemory: Bool) {
    let path = Self.diagnosticSettingsPath()
    settingsPath = path
    diagnosticSettings = true
    // direct ABI 诊断仍用直接构造器，但隐私基准也来自自己的 Caches 配置，绝不读日用配置。
    guard let snapshot = Self.applyDiagnosticPatch(path: path,
      patch: ["memory_enabled":directDiagnosticWithMemory, "privacy_learning_enabled":true]) else { return }
    let cache = InputiaSettingsCache.shared(path: path)
    cache.publish(snapshot)
    settingsCache = cache
    let root = URL(fileURLWithPath: path).deletingLastPathComponent()
    let rime = root.appendingPathComponent("rime").path
    let memory = root.appendingPathComponent("inputia_memory.db").path
    guard (try? InputiaSettingsStore.validateRuntimePaths(snapshot,
      expected: ["rime_user_data_dir":rime, "memory_db_path":memory],
      required: ["rime_user_data_dir", "memory_db_path"])) != nil else { return }
    session = Self.openDirectSession(userDataDir: rime, memoryDbPath: directDiagnosticWithMemory ? memory : nil)
    if session != nil { activeSettings = snapshot }
  }

  static func temporaryDirectForDiagnostics() -> InputiaRustBridge {
    InputiaRustBridge(directDiagnosticWithMemory: false)
  }

  static func temporaryForDiagnostics() -> InputiaRustBridge {
    InputiaRustBridge(directDiagnosticWithMemory: true)
  }

  static func temporarySettingsForDiagnostics() -> InputiaRustBridge {
    InputiaRustBridge(settingsPath: diagnosticSettingsPath(), diagnostics: true)
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

  #if INPUTIA_PAIRED_BUILD
  /// 只回滚Rime近期学习事务，不向宿主发送退格，也不改当前组合快照。
  func undoRecentNativeLearning() -> Bool {
    guard let session, let raw = inputia_session_undo_recent_learning(session) else { return false }
    defer { inputia_string_free(raw) }
    guard let result = Self.parseJsonString(String(cString: raw)), result["ok"] as? Bool == true else { return false }
    return result["requested"] as? Bool == true
  }

  struct PersonalCandidatePool: Decodable {
    let ok: Bool
    let composing: String
    let page: Int
    let candidates: [InputiaPersonalCandidate]
  }
  func personalCandidatePool(limit: Int = 32) -> PersonalCandidatePool? {
    guard let session, let raw = inputia_session_candidate_pool(session, min(64, max(1, limit))) else { return nil }
    defer { inputia_string_free(raw) }
    guard let pool = try? JSONDecoder().decode(PersonalCandidatePool.self, from: Data(String(cString: raw).utf8)),
      pool.ok, pool.composing == latestOutcome.composing, pool.page == latestOutcome.page else { return nil }
    return pool
  }
  func choosePersonalCandidate(id: String, text: String, composing: String) -> InputiaBridgeOutcome {
    guard latestOutcome.composing == composing else { return .failedSelection(preserving: latestOutcome) }
    let raw = composing.withCString { code in id.withCString { candidate in text.withCString { expected in
      inputia_session_choose_candidate_id(session, code, candidate, expected)
    } } }
    guard let raw else { return .failedSelection(preserving: latestOutcome) }
    defer { inputia_string_free(raw) }
    let selection = InputiaBridgeOutcome.decodedSelection(Self.parseJsonString(String(cString: raw)), preserving: latestOutcome)
    guard selection.ok else { return selection }
    latestOutcome = selection
    return selection
  }

  func sharedCandidateOrder(terms: [String]) -> InputiaSharedCandidateOrder? {
    let before = latestOutcome
    guard let session, let data = try? JSONEncoder().encode(terms),
      let json = String(data: data, encoding: .utf8) else { return nil }
    guard let raw = json.withCString({ inputia_session_shared_candidate_order(session, $0) }) else { return nil }
    defer { inputia_string_free(raw) }
    guard latestOutcome.mode == before.mode, latestOutcome.composing == before.composing,
      latestOutcome.page == before.page, latestOutcome.candidates == before.candidates else { return nil }
    return InputiaSharedCandidateOrder.decode(Data(String(cString: raw).utf8), mode: before.mode,
      composing: before.composing, page: before.page, count: before.candidates.count)
  }
  #endif

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
    guard latestOutcome.composing.isEmpty, let state = settingsCache?.state, let snapshot = state.snapshot,
      snapshot.identity != activeSettings?.identity,
      settingsRetry.begin(identity: snapshot.identity, generation: state.generation) else { return false }
    return reloadSettings(snapshot: snapshot)
  }

  func inputModeToggleShortcut() -> String {
    cachedInputModeToggleShortcut
  }

  func scriptToggleShortcut() -> String {
    cachedScriptToggleShortcut
  }

  @discardableResult
  func toggleChineseScriptPreference() -> Bool {
    // Bool 表示快捷键已被认领；保存失败也不能把快捷键泄漏给目标应用。
    guard let cache = settingsCache, let snapshot = cache.state.snapshot else { return true }
    if scriptEdit?.pending == nil { scriptEdit = InputiaSettingsEdit(snapshot) }
    let target = snapshot.values["chinese_script"] as? String == "traditional" ? "simplified" : "traditional"
    do {
      guard let result = try scriptEdit?.apply(path: settingsPath, patch: ["chinese_script": target]) else { return true }
      cache.publish(result.current)
      guard result.status == "saved" else {
        NSLog("Inputia script preference conflicted; selection must be repeated")
        return true
      }
      _ = reloadSettingsIfNeeded()
      return true
    } catch { NSLog("Inputia script preference save failed: \(error.localizedDescription)"); return true }
  }

  func candidateDisplaySettingsApplied(_ snapshot: InputiaSettingsStore.Snapshot) {
    guard candidateDisplayIdentity != snapshot.identity else { return }
    candidateDisplayIdentity = snapshot.identity
    guard snapshot.identity == activeSettings?.identity, let session else { return }
    _ = reportNativeApplication(session: session, snapshot: snapshot)
  }

  private func reportNativeApplication(session: UnsafeMutableRawPointer, snapshot: InputiaSettingsStore.Snapshot) -> Bool {
    var fields = ["input_mode_toggle_shortcut", "script_toggle_shortcut", "shift_toggle_enabled"]
    if candidateDisplayIdentity == snapshot.identity { fields.append("candidate_font_size") }
    return InputiaSettingsStore.reportApplication(session: session, applied: fields)
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

  func setUnverifiedAppContext(bundleId: String) -> Bool {
    guard let session else { return false }
    guard let raw = bundleId.withCString({ inputia_session_set_context_unverified(session, $0) }) else { return false }
    defer { inputia_string_free(raw) }
    return parseJson(raw)?["ok"] as? Bool == true
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

  static func debugSettingsReloadSelfCheck(settingsPath _: String) -> [InputiaBridgeOutcome] {
    let path = diagnosticSettingsPath()
    let bridge = InputiaRustBridge(settingsPath: path, startInChineseMode: true, diagnostics: true)
    var outcomes = [bridge.toggleInputMode()]
    guard let snapshot = applyDiagnosticPatch(path: path, patch: ["shift_toggle_enabled":false,
      "input_mode_toggle_shortcut":"none", "punctuation_preference":"english_in_chinese", "candidate_page_size":7]) else { return [.error] }
    bridge.settingsCache?.publish(snapshot)
    bridge.reloadSettingsIfNeeded()
    outcomes.append(bridge.handleSpecial(keyShift))
    return outcomes
  }

  static func debugCandidateCountFallbackSelfCheck(settingsPath _: String) -> InputiaBridgeOutcome {
    let path = diagnosticSettingsPath()
    let invalidMemory = URL(fileURLWithPath: path).deletingLastPathComponent().appendingPathComponent("inputia_memory.db")
    guard (try? validateDiagnosticSettingsPath(path)) != nil,
      applyDiagnosticPatch(path: path, patch: ["candidate_page_size":8, "schema_id":"double_pinyin",
        "memory_db_path":invalidMemory.path]) != nil,
      (try? FileManager.default.createDirectory(at: invalidMemory, withIntermediateDirectories: true)) != nil else { return .error }
    let bridge = InputiaRustBridge(settingsPath: path, startInChineseMode: true, diagnostics: true)
    var outcome = bridge.latestOutcome
    for character in "yh" { outcome = bridge.handle(character: character) }
    return outcome
  }

  static func debugClipboardPrivacySelfCheck(settingsPath _: String) -> [String: Bool] {
    let path = diagnosticSettingsPath()
    guard applyDiagnosticPatch(path: path, patch: ["memory_enabled":true, "privacy_learning_enabled":true,
      "sensitive_bundle_ids":defaultSensitiveBundleIds]) != nil else { return [:] }
    let bridge = InputiaRustBridge(settingsPath: path, diagnostics: true)
    return ["textedit": bridge.shouldReadClipboard(bundleId: "com.apple.TextEdit"),
      "onepassword": bridge.shouldReadClipboard(bundleId: "com.1password.1password"),
      "unknown": bridge.shouldReadClipboard(bundleId: "unknown"),
      "privateWindow": bridge.shouldReadClipboard(bundleId: "com.apple.Safari", windowTitle: "Private Browsing - Bank Login")]
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
    guard let settings = settingsCache?.state.snapshot?.values, let active = activeSettings?.values,
      active["memory_enabled"] as? Bool == true, active["privacy_learning_enabled"] as? Bool == true else { return false }
    let memoryEnabled = settings["memory_enabled"] as? Bool ?? true
    let privacyLearningEnabled = settings["privacy_learning_enabled"] as? Bool ?? true
    return memoryEnabled && privacyLearningEnabled
  }

  func isSensitiveApp(bundleId: String, windowTitle: String? = nil) -> Bool {
    guard !bundleId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty, bundleId != "unknown" else {
      return true
    }
    guard let settings = settingsCache?.state.snapshot?.values else { return true }
    let sensitiveBundleIds = (settings["sensitive_bundle_ids"] as? [String] ?? Self.defaultSensitiveBundleIds)
      + (activeSettings?.values["sensitive_bundle_ids"] as? [String] ?? Self.defaultSensitiveBundleIds)
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
    guard InputiaProfile.current.allowsHandyImport(path), let session else {
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
    guard InputiaProfile.current.allowsHandyImport(path), let session else {
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

  private func reloadSettings(snapshot: InputiaSettingsStore.Snapshot) -> Bool {
    let previousMode = latestOutcome.mode
    let path = settingsPath, diagnostics = diagnosticSettings
    var opened: (session: UnsafeMutableRawPointer?, withoutMemory: Bool) = (nil, false)
    let replaced = InputiaSettingsSessionSwap.replace(&session, open: {
      opened = Self.openSettingsSession(settingsPath: path, snapshot: snapshot, diagnostics: diagnostics)
      return opened.session
    }, release: inputia_session_free)
    guard replaced, let newSession = session else {
      let report = SettingsApplication(snapshot: snapshot, sessionOpened: false, withoutMemory: false,
        nativeFields: [], failureCode: "session_initialization_failed")
      settingsApplication = report; settingsApplicationDidChange?(report)
      return false
    }
    activeSettings = snapshot
    cachedInputModeToggleShortcut = Self.inputModeToggleShortcut(in: snapshot.values)
    cachedScriptToggleShortcut = Self.scriptToggleShortcut(in: snapshot.values)
    schemaID = snapshot.values["schema_id"] as? String ?? "luna_pinyin_simp"
    let reported = reportNativeApplication(session: newSession, snapshot: snapshot)
    let report = SettingsApplication(snapshot: snapshot, sessionOpened: true, withoutMemory: opened.withoutMemory,
      nativeFields: ["input_mode_toggle_shortcut", "script_toggle_shortcut", "shift_toggle_enabled"],
      failureCode: !reported ? "application_receipt_unavailable" : (opened.withoutMemory ? "memory_unavailable" : nil))
    settingsApplication = report; settingsApplicationDidChange?(report)
    switch previousMode {
    case "Chinese": _ = consume(inputia_session_set_input_mode(session, inputModeChinese))
    case "English": _ = consume(inputia_session_set_input_mode(session, inputModeEnglish))
    default: latestOutcome = .error
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
    InputiaProfile.current.rime.path
  }

  private static func defaultMemoryDbPath() -> String {
    InputiaProfile.current.memory.path
  }

  private static func defaultSettingsPath() -> String {
    InputiaProfile.current.settings.path
  }

  private static func openSettingsSession(settingsPath: String, snapshot: InputiaSettingsStore.Snapshot, diagnostics: Bool)
    -> (session: UnsafeMutableRawPointer?, withoutMemory: Bool) {
    do {
      var expected: [String: String] = [:]
      if diagnostics {
        try validateDiagnosticSettingsPath(settingsPath)
        let root = URL(fileURLWithPath: settingsPath).deletingLastPathComponent()
        expected = ["rime_user_data_dir":root.appendingPathComponent("rime").path,
          "memory_db_path":root.appendingPathComponent("inputia_memory.db").path]
      } else if InputiaProfile.current.isCandidate {
        let isolated = try InputiaProfile.current.isolatedSettings(snapshot.values, settingsPath: settingsPath)
        for key in ["rime_user_data_dir", "memory_db_path", "integration_outbox_path", "integration_snapshot_dir",
          "integration_policy_path", "integration_log_dir", "integration_profile_run_id"] {
          if let value = isolated[key] as? String { expected[key] = value }
        }
      }
      try InputiaSettingsStore.validateRuntimePaths(snapshot, expected: expected,
        required: expected.isEmpty ? [] : ["rime_user_data_dir", "memory_db_path"])
    } catch { return (nil, false) }
    if let session = InputiaSettingsStore.openSession(path: settingsPath, snapshot: snapshot,
      sharedData: bundledRimeDataPath, withoutMemory: false) { return (session, false) }
    let fallback = InputiaSettingsStore.openSession(path: settingsPath, snapshot: snapshot,
      sharedData: bundledRimeDataPath, withoutMemory: true)
    return (fallback, fallback != nil)
  }

  private static func inputModeToggleShortcut(in values: [String: Any]) -> String {
    if let value = values["input_mode_toggle_shortcut"] as? String,
      ["shift", "control_space", "none"].contains(value) { return value }
    return values["shift_toggle_enabled"] as? Bool == false ? "none" : "shift"
  }

  private static func scriptToggleShortcut(in values: [String: Any]) -> String {
    if let value = values["script_toggle_shortcut"] as? String,
      ["control_shift_s", "none"].contains(value) { return value }
    return "control_shift_s"
  }

  private static var diagnosticSettingsRoot: URL {
    FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent("Library/Caches/Inputia/Diagnostics", isDirectory: true)
  }
  private static func diagnosticSettingsPath() -> String {
    diagnosticSettingsRoot.appendingPathComponent(UUID().uuidString.lowercased()).appendingPathComponent("settings.json").path
  }
  private static func validateDiagnosticSettingsPath(_ path: String) throws {
    let url = URL(fileURLWithPath: path)
    guard url.lastPathComponent == "settings.json", UUID(uuidString: url.deletingLastPathComponent().lastPathComponent) != nil,
      url.deletingLastPathComponent().deletingLastPathComponent().path == diagnosticSettingsRoot.path else {
      throw InputiaSettingsStore.Failure(code: "unsafe_path")
    }
  }
  private static func applyDiagnosticPatch(path: String, patch: [String: Any]) -> InputiaSettingsStore.Snapshot? {
    do {
      try validateDiagnosticSettingsPath(path)
      let edit = InputiaSettingsEdit(try InputiaSettingsStore.read(path: path))
      guard let result = try edit.apply(path: path, patch: patch), result.status == "saved" else { return nil }
      return result.current
    } catch { return nil }
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

  private static var bundledRimeDataPath: String? {
    var candidates = [Bundle.main.resourceURL?.appendingPathComponent("RimeData", isDirectory: true)].compactMap { $0 }
    if !InputiaProfile.current.isCandidate {
      candidates.append(URL(fileURLWithPath: "/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/RimeData", isDirectory: true))
    }

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
