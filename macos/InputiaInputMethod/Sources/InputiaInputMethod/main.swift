import Cocoa
import InputMethodKit

private struct InputiaAppContext: Equatable {
  let bundleId: String
  let windowTitle: String?
}

private enum InputiaSecureDirectPolicy {
  private static let secureBundleIds: Set<String> = [
    "com.apple.SecurityAgent",
  ]

  static func shouldUseSecureDirectMode(context: InputiaAppContext) -> Bool {
    secureBundleIds.contains(context.bundleId)
  }
}

private let fallbackBundleIdentifier = "com.inputia.inputmethod.Inputia"
private let connectionName = "com.inputia.inputmethod.Inputia_Connection"
private let emptyReplacementRange = InputiaHostTextPolicy.replacementRange
private let keyCodeDelete: UInt16 = 51
private let keyCodeEscape: UInt16 = 53
private let keyCodePageDown: UInt16 = 121
private let keyCodePageUp: UInt16 = 116
private let keyCodeKeypadEnter: UInt16 = 76
private let keyCodeReturn: UInt16 = 36
private let keyCodeSpace: UInt16 = 49
private let keyCodeTab: UInt16 = 48
private let keyCodePeriod: UInt16 = 47
private let keyCodeDownArrow: UInt16 = 125
private let keyCodeUpArrow: UInt16 = 126

private struct InputiaExpandedCandidateEntry {
  let text: String
  let page: Int
  let pageIndex: Int
}

private func inputiaDebugLog(_ message: String) {
  guard let path = ProcessInfo.processInfo.environment["INPUTIA_DEBUG_EVENTS"] else {
    return
  }
  let line = "\(Date()) \(message)\n"
  guard let data = line.data(using: .utf8) else {
    return
  }
  if !FileManager.default.fileExists(atPath: path) {
    FileManager.default.createFile(atPath: path, contents: nil)
  }
  guard let file = try? FileHandle(forWritingTo: URL(fileURLWithPath: path)) else {
    return
  }
  defer { try? file.close() }
  _ = try? file.seekToEnd()
  _ = try? file.write(contentsOf: data)
}

private func isCurrentInputiaSourceSelected() -> Bool {
  let source = TISCopyCurrentKeyboardInputSource().takeRetainedValue()
  guard
    let rawSourceID = TISGetInputSourceProperty(source, kTISPropertyInputSourceID)
  else {
    return false
  }
  let sourceID = Unmanaged<CFString>.fromOpaque(rawSourceID).takeUnretainedValue() as String
  return sourceID.hasPrefix(fallbackBundleIdentifier)
}

@objc(NSManualApplication)
final class NSManualApplication: NSApplication {}

@objc(InputiaApplicationDelegate)
final class InputiaApplicationDelegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
  var terminateWhenSettingsWindowCloses = false

  func windowWillClose(_ notification: Notification) {
    if terminateWhenSettingsWindowCloses {
      NSApp.terminate(nil)
    }
  }
}

enum InputiaHost {
  static var candidatePanel: InputiaCandidatePanel?
  static var settingsWindowController: InputiaSettingsWindowController?
  static weak var activeInputController: InputiaInputController?
  static var modifierMonitor: Any?
}

@objc(InputiaInputController)
final class InputiaInputController: IMKInputController {
  private let bridge = InputiaRustBridge.makeDefault()
  private var latestCandidates: [String] = []
  private var latestComposing = ""
  private var expandedCandidates: [String] = []
  private var expandedCandidateEntries: [InputiaExpandedCandidateEntry] = []
  private var expandedActiveRowIndex = 0
  private var recallCandidates: [String] = []
  private var englishCompletionPrefix = ""
  private var englishCompletionCandidates: [String] = []
  private var candidatePanelExpanded = false
  private var lastModifiers = NSEvent.ModifierFlags()
  private var lastGlobalModifiers = NSEvent.ModifierFlags()
  private var shiftKeyDownWithoutOtherKey = false
  private var globalShiftKeyDownWithoutOtherKey = false
  private var lastShiftToggleTime = Date.distantPast
  private var cachedAppContext: InputiaAppContext?
  private var cachedAppContextTime = Date.distantPast
  private var pushedAppContext: InputiaAppContext?
  private var lastSettingsReloadCheck = Date.distantPast
  private let appContextRefreshInterval: TimeInterval = 0.75
  private let settingsReloadInterval: TimeInterval = 0.5

  override func handle(_ event: NSEvent!, client sender: Any!) -> Bool {
    guard
      let event,
      let client = sender as? IMKTextInput
    else {
      return false
    }

    if shouldUseSecureDirectMode(client) {
      return false
    }
    updateAppContext(client: client)

    switch event.type {
    case .flagsChanged:
      return handleFlagsChanged(event, client: client)
    case .keyDown:
      return handleKeyDown(event, client: client)
    default:
      return false
    }
  }

  override func recognizedEvents(_ sender: Any!) -> Int {
    Int(NSEvent.EventTypeMask(arrayLiteral: .keyDown, .flagsChanged).rawValue)
  }

  override func candidates(_ sender: Any!) -> [Any]! {
    // Inputia draws its own compact candidate popup. Returning candidate data
    // here makes InputMethodKit show the system IMKCandidates window as well,
    // which follows the user's accent color and can grow into an oversized bar.
    []
  }

  override func menu() -> NSMenu! {
    let voiceInput = NSMenuItem(title: "语音输入", action: #selector(toggleVoiceInput), keyEquivalent: "")
    voiceInput.target = self

    let syncMemory = NSMenuItem(title: "同步语音/剪贴板记忆", action: #selector(syncHandyMemory), keyEquivalent: "")
    syncMemory.target = self

    let recallClipboard = NSMenuItem(
      title: "召回剪贴板",
      action: #selector(recallClipboard),
      keyEquivalent: InputiaHostTextPolicy.recallClipboardMenuKeyEquivalent
    )
    recallClipboard.target = self
    recallClipboard.keyEquivalentModifierMask = [.control, .shift]

    let settings = NSMenuItem(
      title: "Inputia 设置...",
      action: #selector(openSettings),
      keyEquivalent: InputiaHostTextPolicy.settingsMenuKeyEquivalent
    )
    settings.target = self

    let menu = NSMenu()
    menu.addItem(voiceInput)
    menu.addItem(syncMemory)
    menu.addItem(recallClipboard)
    menu.addItem(.separator())
    menu.addItem(settings)
    return menu
  }

  @objc private func toggleVoiceInput() {
    switch InputiaVoiceInputLauncher.triggerVoiceInput() {
    case .started:
      return
    case .missing:
      showHostAlert(
        title: "无法启动语音输入",
        message: "没有找到 Handy.app。请先安装或启动 Handy，再从 Inputia 菜单触发语音输入。"
      )
    case .failed(let message):
      showHostAlert(title: "无法启动语音输入", message: message)
    }
  }

  @objc private func syncHandyMemory() {
    let result = InputiaHandyMemorySync.sync(
      importer: bridge,
      includeHistory: true,
      includeClipboard: true
    )
    showHostAlert(title: "Inputia 记忆同步", message: result.statusText)
  }

  @objc private func recallClipboard() {
    guard let client = client() else {
      return
    }
    _ = showClipboardRecall(client: client)
  }

  @objc private func openSettings() {
    if InputiaHost.settingsWindowController == nil {
      InputiaHost.settingsWindowController = InputiaSettingsWindowController()
    }
    InputiaHost.settingsWindowController?.showWindow(nil)
    NSApp.activate(ignoringOtherApps: true)
  }

  private func showHostAlert(title: String, message: String) {
    let app = NSApplication.shared
    app.setActivationPolicy(.regular)
    app.activate(ignoringOtherApps: true)

    let alert = NSAlert()
    alert.messageText = title
    alert.informativeText = message
    alert.alertStyle = .warning
    alert.addButton(withTitle: "好")
    alert.runModal()
  }

  override func candidateSelected(_ candidateString: NSAttributedString!) {
    guard let selected = candidateString?.string else {
      return
    }
    if englishCompletionCandidates.contains(selected), let client = client() {
      _ = commitEnglishCompletion(selected, client: client)
      return
    }
    if InputiaHostTextPolicy.isRawFallbackSelection(
      selected: selected,
      composing: latestComposing,
      candidates: latestCandidates
    ) {
      _ = apply(bridge.enter(), client: client())
      return
    }
    if candidatePanelExpanded,
      let entry = expandedCandidateEntries.first(where: { $0.text == selected })
    {
      _ = commitExpandedCandidate(entry, client: client())
      return
    }
    guard let index = latestCandidates.firstIndex(of: selected) else {
      return
    }
    _ = apply(bridge.chooseCandidate(atZeroBasedIndex: index), client: client())
  }

  override func commitComposition(_ sender: Any!) {
    guard let client = (sender as? IMKTextInput) ?? client() else {
      return
    }
    let context = appContext(for: client)
    if bridge.isSensitiveApp(bundleId: context.bundleId, windowTitle: context.windowTitle) {
      clearInputState(client: client)
      return
    }
    if !latestComposing.isEmpty {
      let previousComposing = latestComposing
      let outcome = bridge.enter()
      if outcome.commit?.isEmpty == false {
        _ = apply(outcome, client: client)
        return
      }
      insertCommittedText(
        latestComposing,
        client: client,
        replacementRange: InputiaHostTextPolicy.commitReplacementRange(
          previousComposing: previousComposing,
          markedRange: client.markedRange()
        )
      )
      _ = bridge.escape()
    }
    latestComposing = ""
    latestCandidates = []
    englishCompletionPrefix = ""
    englishCompletionCandidates = []
    expandedCandidates = []
    expandedCandidateEntries = []
    expandedActiveRowIndex = 0
    candidatePanelExpanded = false
    InputiaHost.candidatePanel?.hide()
  }

  override func hidePalettes() {
    InputiaHost.candidatePanel?.hide()
    super.hidePalettes()
  }

  override func activateServer(_ sender: Any!) {
    InputiaHost.activeInputController = self
    if let client = sender as? IMKTextInput {
      if shouldUseSecureDirectMode(client) {
        return
      }
      updateAppContext(client: client, forceRefresh: true)
      resetToChineseModeOnActivationIfNeeded(client: client)
    }
  }

  override func deactivateServer(_ sender: Any!) {
    commitComposition(sender)
    if InputiaHost.activeInputController === self {
      InputiaHost.activeInputController = nil
    }
  }

  func handleGlobalFlagsChanged(_ event: NSEvent) {
    guard isCurrentInputiaSourceSelected() else {
      globalShiftKeyDownWithoutOtherKey = false
      lastGlobalModifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
      return
    }

    let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
    let hadShift = lastGlobalModifiers.contains(.shift)
    let hasShift = modifiers.contains(.shift)
    let hasBlockingModifier = modifiers.contains(.command)
      || modifiers.contains(.control)
      || modifiers.contains(.option)
    let shortcut = bridge.inputModeToggleShortcut()

    inputiaDebugLog(
      "globalFlagsChanged keyCode=\(event.keyCode) last=\(lastGlobalModifiers.rawValue) current=\(modifiers.rawValue) hadShift=\(hadShift) hasShift=\(hasShift) blocking=\(hasBlockingModifier) shortcut=\(shortcut) armed=\(globalShiftKeyDownWithoutOtherKey)"
    )

    if !hadShift && hasShift {
      globalShiftKeyDownWithoutOtherKey = InputiaShortcutClassifier.shouldArmShiftInputModeToggle(
        shortcut: shortcut,
        modifiers: modifiers
      )
    } else if hadShift && !hasShift {
      defer {
        globalShiftKeyDownWithoutOtherKey = false
        lastGlobalModifiers = modifiers
      }
      if InputiaShortcutClassifier.isShiftInputModeToggleRelease(
        shortcut: shortcut,
        hadShift: hadShift,
        hasShift: hasShift,
        hasBlockingModifier: hasBlockingModifier,
        armed: globalShiftKeyDownWithoutOtherKey
      ) {
        _ = toggleInputModeFromShift(client: nil, source: "global")
      }
      return
    }

    lastGlobalModifiers = modifiers
  }

  private func handleFlagsChanged(_ event: NSEvent, client: IMKTextInput) -> Bool {
    reloadSettingsIfDue(client: client)
    let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
    let hadShift = lastModifiers.contains(.shift)
    let hasShift = modifiers.contains(.shift)
    let hasBlockingModifier = modifiers.contains(.command)
      || modifiers.contains(.control)
      || modifiers.contains(.option)
    let shortcut = bridge.inputModeToggleShortcut()

    inputiaDebugLog(
      "flagsChanged keyCode=\(event.keyCode) last=\(lastModifiers.rawValue) current=\(modifiers.rawValue) hadShift=\(hadShift) hasShift=\(hasShift) blocking=\(hasBlockingModifier) shortcut=\(shortcut) armed=\(shiftKeyDownWithoutOtherKey)"
    )

    if !hadShift && hasShift {
      shiftKeyDownWithoutOtherKey = InputiaShortcutClassifier.shouldArmShiftInputModeToggle(
        shortcut: shortcut,
        modifiers: modifiers
      )
    } else if hadShift && !hasShift {
      defer {
        shiftKeyDownWithoutOtherKey = false
        lastModifiers = modifiers
      }
      if InputiaShortcutClassifier.isShiftInputModeToggleRelease(
        shortcut: shortcut,
        hadShift: hadShift,
        hasShift: hasShift,
        hasBlockingModifier: hasBlockingModifier,
        armed: shiftKeyDownWithoutOtherKey
      ) {
        return toggleInputModeFromShift(client: client, source: "local")
      }
    }

    lastModifiers = modifiers
    return false
  }

  private func toggleInputModeFromShift(client: IMKTextInput?, source: String) -> Bool {
    let now = Date()
    guard now.timeIntervalSince(lastShiftToggleTime) >= 0.18 else {
      inputiaDebugLog("shiftToggleDeduped source=\(source)")
      return true
    }
    lastShiftToggleTime = now
    clearEnglishCompletion()
    inputiaDebugLog("shiftToggle source=\(source)")
    return apply(bridge.toggleInputMode(), client: client)
  }

  private func resetToChineseModeOnActivationIfNeeded(client: IMKTextInput) {
    guard latestComposing.isEmpty, bridge.latestOutcome.mode == "English" else {
      return
    }
    clearEnglishCompletion()
    let outcome = bridge.setChineseMode()
    inputiaDebugLog("activateResetChinese bundle=\(client.bundleIdentifier() ?? "unknown")")
    _ = apply(outcome, client: client)
  }

  private func handleKeyDown(_ event: NSEvent, client: IMKTextInput) -> Bool {
    let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
    inputiaDebugLog(
      "keyDown keyCode=\(event.keyCode) modifiers=\(modifiers.rawValue) chars=\(event.characters ?? "") charsIgnoring=\(event.charactersIgnoringModifiers ?? "")"
    )
    if isScriptToggleShortcut(event, modifiers: modifiers) {
      shiftKeyDownWithoutOtherKey = false
      clearEnglishCompletion()
      clearInputState(client: client)
      return bridge.toggleChineseScriptPreference()
    }
    if isClipboardRecallShortcut(event, modifiers: modifiers) {
      shiftKeyDownWithoutOtherKey = false
      return showClipboardRecall(client: client)
    }
    if !recallCandidates.isEmpty {
      if handleRecallKeyDown(event, client: client) {
        return true
      }
    }
    if isPunctuationToggleShortcut(event, modifiers: modifiers) {
      shiftKeyDownWithoutOtherKey = false
      clearEnglishCompletion()
      return apply(bridge.togglePunctuationPreference(), client: client)
    }
    if isCharacterWidthToggleShortcut(event, modifiers: modifiers) {
      shiftKeyDownWithoutOtherKey = false
      clearEnglishCompletion()
      return apply(bridge.toggleCharacterWidthPreference(), client: client)
    }
    if isInputModeToggleShortcut(event, modifiers: modifiers) {
      shiftKeyDownWithoutOtherKey = false
      clearEnglishCompletion()
      return apply(bridge.toggleInputMode(), client: client)
    }
    if let navigation = InputiaShortcutClassifier.candidateNavigation(
      keyCode: event.keyCode,
      modifiers: modifiers,
      hasComposing: !latestComposing.isEmpty
    ) {
      return handleCandidateNavigation(navigation, client: client)
    }
    if modifiers.contains(.command) || modifiers.contains(.control) || modifiers.contains(.option) {
      shiftKeyDownWithoutOtherKey = false
      return false
    }
    if modifiers.contains(.shift) {
      shiftKeyDownWithoutOtherKey = false
    }

    switch event.keyCode {
    case keyCodeDelete:
      let outcome = bridge.backspace()
      let handled = apply(outcome, client: client)
      updateEnglishCompletionAfterBackspace(outcome: outcome, client: client)
      return handled
    case keyCodeEscape:
      if latestComposing.isEmpty && !englishCompletionCandidates.isEmpty {
        clearEnglishCompletion()
        return true
      }
      clearEnglishCompletion()
      return apply(bridge.escape(), client: client)
    case keyCodePageDown:
      return handleCandidatePageDown(client: client)
    case keyCodePageUp:
      return handleCandidatePageUp(client: client)
    case keyCodeReturn, keyCodeKeypadEnter:
      guard !latestComposing.isEmpty else {
        clearEnglishCompletion()
        return false
      }
      let previousComposing = latestComposing
      let previousCandidates = latestCandidates
      let outcome = bridge.enter()
      let shouldPassThroughNewline = InputiaHostTextPolicy.shouldPassThroughNewlineAfterRawFallbackCommit(
        previousComposing: previousComposing,
        candidates: previousCandidates,
        committedText: outcome.commit
      )
      let handled = apply(outcome, client: client)
      if outcome.mode == "English" && !outcome.consumed {
        learnAndClearEnglishCompletion(client: client)
      }
      return shouldPassThroughNewline ? false : handled
    case keyCodeSpace:
      if candidatePanelExpanded, expandedActiveRowIndex > 0 {
        return commitExpandedCandidate(columnIndex: 0, client: client)
      }
      let outcome = bridge.space()
      let handled = apply(outcome, client: client)
      if outcome.mode == "English" && !outcome.consumed {
        learnAndClearEnglishCompletion(client: client)
      }
      return handled
    case keyCodeTab:
      return commitFirstEnglishCompletion(client: client)
    default:
      break
    }

    guard let text = event.characters, !text.isEmpty else {
      return false
    }

    if isDisplayedRawCompositionSelection(event, modifiers: modifiers) {
      return apply(bridge.enter(), client: client)
    }

    if let columnIndex = expandedCandidateDigitColumn(event, modifiers: modifiers) {
      return commitExpandedCandidate(columnIndex: columnIndex, client: client)
    }

    var handled = false
    for character in text {
      let outcome = bridge.handle(character: character)
      handled = apply(outcome, client: client) || handled
      updateEnglishCompletionAfterCharacter(character, outcome: outcome, client: client)
    }
    return handled
  }

  private func apply(_ outcome: InputiaBridgeOutcome, client: IMKTextInput?) -> Bool {
    clearClipboardRecall()
    let previousComposing = latestComposing
    inputiaDebugLog(
      "apply ok=\(outcome.ok) consumed=\(outcome.consumed) mode=\(outcome.mode) composing=\(outcome.composing) commit=\(outcome.commit ?? "") candidates=\(outcome.candidates.prefix(3).joined(separator: ","))"
    )
    guard outcome.ok else {
      NSLog("Inputia bridge outcome error")
      return false
    }
    guard let client else {
      syncHostState(with: outcome)
      return outcome.consumed
    }

    if let commit = outcome.commit, !commit.isEmpty {
      candidatePanelExpanded = false
      syncHostState(with: outcome)
      insertCommittedText(
        commit,
        client: client,
        replacementRange: InputiaHostTextPolicy.commitReplacementRange(
          previousComposing: previousComposing,
          markedRange: client.markedRange()
        )
      )
      if InputiaHostTextPolicy.shouldContinueMarkedTextAfterCommit(
        committedText: outcome.commit,
        nextComposing: outcome.composing
      ) {
        setMarkedComposition(outcome.composing, client: client)
        updateCandidateWindow(client: client)
      } else {
        InputiaHost.candidatePanel?.hide()
      }
      return outcome.consumed
    }

    syncHostState(with: outcome)
    if outcome.composing.isEmpty || outcome.composing != previousComposing {
      candidatePanelExpanded = false
    }
    if InputiaHostTextPolicy.shouldClearMarkedText(
      previousComposing: previousComposing,
      nextComposing: outcome.composing,
      committedText: outcome.commit
    ) {
      clearMarkedText(client)
    }

    if outcome.composing.isEmpty {
      candidatePanelExpanded = false
      InputiaHost.candidatePanel?.hide()
    } else {
      if outcome.composing != previousComposing {
        setMarkedComposition(outcome.composing, client: client)
      }
      updateCandidateWindow(client: client)
    }

    return outcome.consumed
  }

  private func syncHostState(with outcome: InputiaBridgeOutcome) {
    let compositionChanged = latestComposing != outcome.composing
    latestComposing = outcome.composing
    latestCandidates = outcome.candidates
    if compositionChanged || !candidatePanelExpanded {
      expandedCandidates = []
      expandedCandidateEntries = []
      expandedActiveRowIndex = 0
    }
    if outcome.mode != "English" {
      englishCompletionPrefix = ""
      englishCompletionCandidates = []
    }
  }

  private func insertCommittedText(
    _ text: String,
    client: IMKTextInput,
    replacementRange: NSRange = emptyReplacementRange
  ) {
    client.insertText(text, replacementRange: replacementRange)
  }

  private func handleCandidateNavigation(
    _ navigation: InputiaCandidateNavigation,
    client: IMKTextInput
  ) -> Bool {
    guard !latestComposing.isEmpty else {
      return false
    }
    shiftKeyDownWithoutOtherKey = false
    clearEnglishCompletion()

    switch navigation {
    case .expandOrNextPage:
      guard candidatePanelExpanded else {
        candidatePanelExpanded = true
        expandedActiveRowIndex = 0
        refreshExpandedCandidates()
        updateCandidateWindow(client: client)
        inputiaDebugLog("candidatePanelExpanded")
        return true
      }
      if moveExpandedActiveRow(by: 1, client: client) {
        return true
      }
      return handleCandidatePageDown(client: client)
    case .previousPage:
      if candidatePanelExpanded,
        let previousRow = InputiaExpandedCandidateGridNavigation.previousRow(
          currentRow: expandedActiveRowIndex
        )
      {
        expandedActiveRowIndex = previousRow
        updateCandidateWindow(client: client)
        inputiaDebugLog("candidatePanelActiveRow=\(expandedActiveRowIndex)")
        return true
      }
      if candidatePanelExpanded, bridge.latestOutcome.page == 0 {
        candidatePanelExpanded = false
        expandedCandidates = []
        expandedCandidateEntries = []
        expandedActiveRowIndex = 0
        updateCandidateWindow(client: client)
        inputiaDebugLog("candidatePanelCollapsed")
        return true
      }
      return handleCandidatePageUp(client: client)
    }
  }

  private func handleCandidatePageDown(client: IMKTextInput) -> Bool {
    guard !latestComposing.isEmpty else {
      return false
    }
    candidatePanelExpanded = true
    expandedCandidates = []
    expandedCandidateEntries = []
    expandedActiveRowIndex = 0
    let handled = apply(bridge.pageDown(), client: client)
    if !latestComposing.isEmpty {
      refreshExpandedCandidates()
      updateCandidateWindow(client: client)
    }
    return handled || !latestComposing.isEmpty
  }

  private func handleCandidatePageUp(client: IMKTextInput) -> Bool {
    guard !latestComposing.isEmpty else {
      return false
    }
    candidatePanelExpanded = true
    expandedCandidates = []
    expandedCandidateEntries = []
    expandedActiveRowIndex = 0
    let handled = apply(bridge.pageUp(), client: client)
    if !latestComposing.isEmpty {
      refreshExpandedCandidates()
      updateCandidateWindow(client: client)
    }
    return handled || !latestComposing.isEmpty
  }

  private func refreshExpandedCandidates(targetCount: Int = 40) {
    guard candidatePanelExpanded, !latestComposing.isEmpty else {
      expandedCandidates = []
      expandedCandidateEntries = []
      expandedActiveRowIndex = 0
      return
    }
    expandedCandidateEntries = collectExpandedCandidateEntries(targetCount: targetCount)
    expandedCandidates = expandedCandidateEntries.map(\.text)
    clampExpandedActiveRow()
  }

  private func collectExpandedCandidateEntries(targetCount: Int) -> [InputiaExpandedCandidateEntry] {
    var collected: [InputiaExpandedCandidateEntry] = []
    var seen = Set<String>()

    func appendUnique(_ candidates: [String], page: Int) {
      for (pageIndex, candidate) in candidates.enumerated() where collected.count < targetCount {
        guard InputiaCandidateTextSupport.canDisplay(candidate) else {
          continue
        }
        if seen.insert(candidate).inserted {
          collected.append(
            InputiaExpandedCandidateEntry(text: candidate, page: page, pageIndex: pageIndex)
          )
        }
      }
    }

    appendUnique(latestCandidates, page: bridge.latestOutcome.page)

    let originalPage = bridge.latestOutcome.page
    var lastPage = originalPage
    var movedPages = 0

    while collected.count < targetCount {
      let outcome = bridge.pageDown()
      guard outcome.ok, outcome.composing == latestComposing, outcome.page > lastPage else {
        break
      }
      movedPages += 1
      lastPage = outcome.page
      let beforeCount = collected.count
      appendUnique(outcome.candidates, page: outcome.page)
      if collected.count == beforeCount && outcome.candidates.isEmpty {
        break
      }
    }

    for _ in 0..<movedPages {
      _ = bridge.pageUp()
    }

    return collected
  }

  private func expandedColumnCount() -> Int {
    max(1, InputiaHostTextPolicy.candidatesForPanel(
      composing: latestComposing,
      candidates: latestCandidates
    ).count)
  }

  private func expandedRowCount() -> Int {
    InputiaExpandedCandidateGridNavigation.rowCount(
      candidateCount: expandedCandidateEntries.count,
      columnCount: expandedColumnCount()
    )
  }

  private func clampExpandedActiveRow() {
    expandedActiveRowIndex = InputiaExpandedCandidateGridNavigation.clampedRow(
      expandedActiveRowIndex,
      candidateCount: expandedCandidateEntries.count,
      columnCount: expandedColumnCount()
    )
  }

  private func moveExpandedActiveRow(by delta: Int, client: IMKTextInput) -> Bool {
    guard candidatePanelExpanded, !expandedCandidateEntries.isEmpty, delta != 0 else {
      return false
    }
    let columns = expandedColumnCount()
    let nextRow: Int?
    if delta > 0 {
      nextRow = InputiaExpandedCandidateGridNavigation.nextRow(
        currentRow: expandedActiveRowIndex,
        candidateCount: expandedCandidateEntries.count,
        columnCount: columns
      )
    } else {
      nextRow = InputiaExpandedCandidateGridNavigation.previousRow(currentRow: expandedActiveRowIndex)
    }
    guard let nextRow else {
      return false
    }
    expandedActiveRowIndex = nextRow
    updateCandidateWindow(client: client)
    inputiaDebugLog("candidatePanelActiveRow=\(expandedActiveRowIndex)")
    return true
  }

  private func expandedCandidateDigitColumn(
    _ event: NSEvent,
    modifiers: NSEvent.ModifierFlags
  ) -> Int? {
    guard candidatePanelExpanded, !expandedCandidateEntries.isEmpty else {
      return nil
    }
    guard !modifiers.contains(.command),
      !modifiers.contains(.control),
      !modifiers.contains(.option),
      !modifiers.contains(.shift)
    else {
      return nil
    }
    guard
      let raw = event.charactersIgnoringModifiers,
      raw.count == 1,
      let digit = Int(raw),
      (1...9).contains(digit)
    else {
      return nil
    }
    return digit - 1
  }

  private func commitExpandedCandidate(columnIndex: Int, client: IMKTextInput?) -> Bool {
    let index = expandedActiveRowIndex * expandedColumnCount() + columnIndex
    guard expandedCandidateEntries.indices.contains(index) else {
      return false
    }
    return commitExpandedCandidate(expandedCandidateEntries[index], client: client)
  }

  private func commitExpandedCandidate(_ entry: InputiaExpandedCandidateEntry, client: IMKTextInput?) -> Bool {
    guard moveBridgeToPage(entry.page) else {
      return false
    }
    return apply(bridge.chooseCandidate(atZeroBasedIndex: entry.pageIndex), client: client)
  }

  private func moveBridgeToPage(_ targetPage: Int) -> Bool {
    var guardCount = 0
    while bridge.latestOutcome.page < targetPage, guardCount < 32 {
      let previousPage = bridge.latestOutcome.page
      let outcome = bridge.pageDown()
      guard outcome.ok, outcome.page > previousPage else {
        return false
      }
      guardCount += 1
    }
    while bridge.latestOutcome.page > targetPage, guardCount < 64 {
      let previousPage = bridge.latestOutcome.page
      let outcome = bridge.pageUp()
      guard outcome.ok, outcome.page < previousPage || previousPage == 0 else {
        return false
      }
      guardCount += 1
    }
    return bridge.latestOutcome.page == targetPage
  }

  private func setMarkedComposition(_ composing: String, client: IMKTextInput) {
    client.setMarkedText(
      composing,
      selectionRange: NSRange(location: composing.utf16.count, length: 0),
      replacementRange: emptyReplacementRange
    )
  }

  private func clearMarkedText(_ client: IMKTextInput) {
    client.setMarkedText(
      "",
      selectionRange: NSRange(location: 0, length: 0),
      replacementRange: emptyReplacementRange
    )
  }

  private func isClipboardRecallShortcut(_ event: NSEvent, modifiers: NSEvent.ModifierFlags) -> Bool {
    InputiaShortcutClassifier.isClipboardRecall(
      charactersIgnoringModifiers: event.charactersIgnoringModifiers,
      modifiers: modifiers
    )
  }

  private func isScriptToggleShortcut(_ event: NSEvent, modifiers: NSEvent.ModifierFlags) -> Bool {
    InputiaShortcutClassifier.isScriptToggle(
      charactersIgnoringModifiers: event.charactersIgnoringModifiers,
      modifiers: modifiers,
      shortcut: bridge.scriptToggleShortcut()
    )
  }

  private func isPunctuationToggleShortcut(
    _ event: NSEvent,
    modifiers: NSEvent.ModifierFlags
  ) -> Bool {
    InputiaShortcutClassifier.isPunctuationToggle(
      keyCode: event.keyCode,
      charactersIgnoringModifiers: event.charactersIgnoringModifiers,
      modifiers: modifiers
    )
  }

  private func isCharacterWidthToggleShortcut(
    _ event: NSEvent,
    modifiers: NSEvent.ModifierFlags
  ) -> Bool {
    InputiaShortcutClassifier.isCharacterWidthToggle(
      keyCode: event.keyCode,
      modifiers: modifiers
    )
  }

  private func isInputModeToggleShortcut(
    _ event: NSEvent,
    modifiers: NSEvent.ModifierFlags
  ) -> Bool {
    InputiaShortcutClassifier.isControlSpaceInputModeToggle(
      keyCode: event.keyCode,
      modifiers: modifiers,
      shortcut: bridge.inputModeToggleShortcut()
    )
  }

  private func isDisplayedRawCompositionSelection(
    _ event: NSEvent,
    modifiers: NSEvent.ModifierFlags
  ) -> Bool {
    InputiaShortcutClassifier.isDisplayedRawCompositionSelection(
      characters: event.characters,
      charactersIgnoringModifiers: event.charactersIgnoringModifiers,
      modifiers: modifiers,
      hasComposing: !latestComposing.isEmpty,
      hasCandidates: !latestCandidates.isEmpty
    )
  }

  private func showClipboardRecall(client: IMKTextInput) -> Bool {
    let context = appContext(for: client)
    guard bridge.shouldReadClipboard(bundleId: context.bundleId, windowTitle: context.windowTitle) else {
      inputiaDebugLog("clipboardRecallSkipped bundle=\(context.bundleId) window=\(context.windowTitle ?? "")")
      clearClipboardRecall()
      return false
    }
    learnAndClearEnglishCompletion(client: client)

    if let clipboardText = NSPasteboard.general.string(forType: .string) {
      let normalizedText = clipboardText.trimmingCharacters(in: .whitespacesAndNewlines)
      if !normalizedText.isEmpty {
        _ = bridge.learnClipboard(
          text: normalizedText,
          bundleId: context.bundleId,
          windowTitle: context.windowTitle
        )
      }
    }

    let candidates = bridge.clipboardCandidates(limit: 9)
    guard !candidates.isEmpty else {
      clearClipboardRecall()
      return false
    }

    recallCandidates = candidates
    latestComposing = ""
    latestCandidates = candidates
    candidatePanelExpanded = false
    expandedCandidates = []
    expandedCandidateEntries = []
    expandedActiveRowIndex = 0

    var inputRect = NSRect.zero
    client.attributes(forCharacterIndex: 0, lineHeightRectangle: &inputRect)
    InputiaHost.candidatePanel?.show(candidates: candidates, near: inputRect)
    inputiaDebugLog("clipboardRecallShown count=\(candidates.count)")
    return true
  }

  private func handleRecallKeyDown(_ event: NSEvent, client: IMKTextInput) -> Bool {
    switch event.keyCode {
    case keyCodeEscape, keyCodeDelete:
      clearClipboardRecall()
      return true
    case keyCodeReturn, keyCodeSpace:
      return commitRecallCandidate(atZeroBasedIndex: 0, client: client)
    default:
      break
    }

    if
      let text = event.charactersIgnoringModifiers,
      text.count == 1,
      let digit = Int(text),
      (1...9).contains(digit)
    {
      return commitRecallCandidate(atZeroBasedIndex: digit - 1, client: client)
    }

    clearClipboardRecall()
    return false
  }

  private func commitRecallCandidate(atZeroBasedIndex index: Int, client: IMKTextInput) -> Bool {
    guard recallCandidates.indices.contains(index) else {
      return false
    }
    let text = recallCandidates[index]
    client.insertText(text, replacementRange: emptyReplacementRange)
    inputiaDebugLog("clipboardRecallCommit index=\(index) text=\(text)")
    clearClipboardRecall()
    return true
  }

  private func clearClipboardRecall() {
    guard !recallCandidates.isEmpty else {
      return
    }
    recallCandidates = []
    latestCandidates = []
    latestComposing = ""
    candidatePanelExpanded = false
    expandedCandidates = []
    expandedCandidateEntries = []
    expandedActiveRowIndex = 0
    InputiaHost.candidatePanel?.hide()
  }

  private func updateEnglishCompletionAfterCharacter(
    _ character: Character,
    outcome: InputiaBridgeOutcome,
    client: IMKTextInput
  ) {
    guard outcome.ok, outcome.mode == "English", latestComposing.isEmpty else {
      clearEnglishCompletion()
      return
    }
    guard outcome.commit == String(character) else {
      return
    }

    if isEnglishWordCharacter(character) {
      englishCompletionPrefix.append(character)
      refreshEnglishCompletions(client: client)
    } else {
      learnAndClearEnglishCompletion(client: client)
    }
  }

  private func updateEnglishCompletionAfterBackspace(outcome: InputiaBridgeOutcome, client: IMKTextInput) {
    guard outcome.ok, outcome.mode == "English", latestComposing.isEmpty else {
      clearEnglishCompletion()
      return
    }
    guard !englishCompletionPrefix.isEmpty else {
      hideEnglishCompletionCandidates()
      return
    }
    englishCompletionPrefix.removeLast()
    refreshEnglishCompletions(client: client)
  }

  private func refreshEnglishCompletions(client: IMKTextInput) {
    guard englishCompletionPrefix.count >= 2 else {
      hideEnglishCompletionCandidates()
      return
    }
    let candidates = bridge.completionCandidates(prefix: englishCompletionPrefix, limit: 5)
      .filter { completionSuffix(for: $0) != nil }
    guard !candidates.isEmpty else {
      hideEnglishCompletionCandidates()
      return
    }

    englishCompletionCandidates = candidates
    latestCandidates = candidates
    candidatePanelExpanded = false
    expandedCandidates = []
    expandedCandidateEntries = []
    expandedActiveRowIndex = 0

    var inputRect = NSRect.zero
    client.attributes(forCharacterIndex: 0, lineHeightRectangle: &inputRect)
    InputiaHost.candidatePanel?.show(candidates: candidates, near: inputRect)
    inputiaDebugLog("englishCompletionShown prefix=\(englishCompletionPrefix) count=\(candidates.count)")
  }

  private func commitFirstEnglishCompletion(client: IMKTextInput) -> Bool {
    guard let candidate = englishCompletionCandidates.first else {
      return false
    }
    return commitEnglishCompletion(candidate, client: client)
  }

  private func commitEnglishCompletion(_ candidate: String, client: IMKTextInput) -> Bool {
    guard let suffix = completionSuffix(for: candidate), !suffix.isEmpty else {
      clearEnglishCompletion()
      return false
    }
    client.insertText(suffix, replacementRange: emptyReplacementRange)
    let context = appContext(for: client)
    _ = bridge.learnTyped(text: candidate, bundleId: context.bundleId, windowTitle: context.windowTitle)
    inputiaDebugLog("englishCompletionCommit prefix=\(englishCompletionPrefix) candidate=\(candidate)")
    clearEnglishCompletion()
    return true
  }

  private func completionSuffix(for candidate: String) -> String? {
    let prefix = englishCompletionPrefix
    guard !prefix.isEmpty else {
      return nil
    }
    guard candidate.lowercased().hasPrefix(prefix.lowercased()) else {
      return nil
    }
    guard candidate.count > prefix.count else {
      return nil
    }
    let start = candidate.index(candidate.startIndex, offsetBy: prefix.count)
    return String(candidate[start...])
  }

  private func learnAndClearEnglishCompletion(client: IMKTextInput) {
    let word = englishCompletionPrefix
    if isLearnableEnglishWord(word) {
      let context = appContext(for: client)
      _ = bridge.learnTyped(text: word, bundleId: context.bundleId, windowTitle: context.windowTitle)
      inputiaDebugLog("englishWordLearned word=\(word)")
    }
    clearEnglishCompletion()
  }

  private func hideEnglishCompletionCandidates() {
    englishCompletionCandidates = []
    if latestComposing.isEmpty && recallCandidates.isEmpty {
      latestCandidates = []
      expandedCandidates = []
      expandedCandidateEntries = []
      expandedActiveRowIndex = 0
      candidatePanelExpanded = false
      InputiaHost.candidatePanel?.hide()
    }
  }

  private func clearEnglishCompletion() {
    englishCompletionPrefix = ""
    hideEnglishCompletionCandidates()
  }

  private func isLearnableEnglishWord(_ word: String) -> Bool {
    word.count >= 2 && word.unicodeScalars.contains { scalar in
      (65...90).contains(scalar.value) || (97...122).contains(scalar.value)
    }
  }

  private func isEnglishWordCharacter(_ character: Character) -> Bool {
    character.unicodeScalars.allSatisfy { scalar in
      (48...57).contains(scalar.value)
        || (65...90).contains(scalar.value)
        || (97...122).contains(scalar.value)
        || scalar.value == 95
        || scalar.value == 45
    }
  }

  private func clearInputState(client: IMKTextInput? = nil) {
    if !latestComposing.isEmpty, let client {
      clearMarkedText(client)
    }
    recallCandidates = []
    latestCandidates = []
    latestComposing = ""
    englishCompletionPrefix = ""
    englishCompletionCandidates = []
    expandedCandidates = []
    expandedCandidateEntries = []
    expandedActiveRowIndex = 0
    candidatePanelExpanded = false
    _ = bridge.escape()
    InputiaHost.candidatePanel?.hide()
  }

  private func updateCandidateWindow(client: IMKTextInput) {
    guard let panel = InputiaHost.candidatePanel else {
      return
    }
    let displayedCandidates = InputiaHostTextPolicy.candidatesForPanel(
      composing: latestComposing,
      candidates: latestCandidates
    )
    let panelCandidates = candidatePanelExpanded && !expandedCandidates.isEmpty
      ? expandedCandidates
      : displayedCandidates
    if panelCandidates.isEmpty {
      panel.hide()
      return
    }

    var inputRect = NSRect.zero
    client.attributes(forCharacterIndex: 0, lineHeightRectangle: &inputRect)
    panel.show(
      candidates: panelCandidates,
      near: inputRect,
      expanded: candidatePanelExpanded,
      primaryCandidateCount: displayedCandidates.count,
      activeRowIndex: expandedActiveRowIndex
    )
  }

  private func updateAppContext(client: IMKTextInput, forceRefresh: Bool = false) {
    reloadSettingsIfDue(client: client, force: forceRefresh)
    let context = appContext(for: client, forceRefresh: forceRefresh)
    inputiaDebugLog("context bundle=\(context.bundleId) window=\(context.windowTitle ?? "")")
    guard pushedAppContext != context else {
      return
    }
    _ = bridge.setAppContext(bundleId: context.bundleId, windowTitle: context.windowTitle)
    pushedAppContext = context
  }

  private func shouldUseSecureDirectMode(_ client: IMKTextInput) -> Bool {
    reloadSettingsIfDue(client: client)
    let context = appContext(for: client)
    guard InputiaSecureDirectPolicy.shouldUseSecureDirectMode(context: context) else {
      return false
    }
    clearInputState(client: client)
    inputiaDebugLog("secureDirectPassthrough bundle=\(context.bundleId) window=\(context.windowTitle ?? "")")
    if pushedAppContext != context {
      _ = bridge.setAppContext(bundleId: context.bundleId, windowTitle: context.windowTitle)
      pushedAppContext = context
    }
    return true
  }

  private func appContext(for client: IMKTextInput, forceRefresh: Bool = false) -> InputiaAppContext {
    let bundleId = resolvedBundleIdentifier(for: client)
    let now = Date()
    if
      !forceRefresh,
      let cachedAppContext,
      cachedAppContext.bundleId == bundleId,
      now.timeIntervalSince(cachedAppContextTime) < appContextRefreshInterval
    {
      return cachedAppContext
    }

    let context = InputiaAppContext(
      bundleId: bundleId,
      windowTitle: activeWindowTitle(forBundleId: bundleId)
    )
    cachedAppContext = context
    cachedAppContextTime = now
    return context
  }

  private func resolvedBundleIdentifier(for client: IMKTextInput) -> String {
    if let clientBundleId = client.bundleIdentifier()?.trimmingCharacters(in: .whitespacesAndNewlines),
      !clientBundleId.isEmpty,
      clientBundleId != "unknown"
    {
      return clientBundleId
    }
    if let frontmostBundleId = NSWorkspace.shared.frontmostApplication?.bundleIdentifier?
      .trimmingCharacters(in: .whitespacesAndNewlines),
      !frontmostBundleId.isEmpty
    {
      return frontmostBundleId
    }
    return "unknown"
  }

  private func reloadSettingsIfDue(client: IMKTextInput? = nil, force: Bool = false) {
    let now = Date()
    guard force || now.timeIntervalSince(lastSettingsReloadCheck) >= settingsReloadInterval else {
      return
    }
    lastSettingsReloadCheck = now
    if bridge.reloadSettingsIfNeeded() {
      clearInputState(client: client)
      pushedAppContext = nil
    }
  }

  private func activeWindowTitle(forBundleId bundleId: String) -> String? {
    guard
      let frontmost = NSWorkspace.shared.frontmostApplication,
      frontmost.bundleIdentifier == bundleId
    else {
      return nil
    }
    let processIdentifier = frontmost.processIdentifier
    guard
      let windowList = CGWindowListCopyWindowInfo(
        [.optionOnScreenOnly, .excludeDesktopElements],
        kCGNullWindowID
      ) as? [[String: Any]]
    else {
      return nil
    }
    for window in windowList {
      let ownerPid = (window[kCGWindowOwnerPID as String] as? pid_t)
        ?? (window[kCGWindowOwnerPID as String] as? Int).map(pid_t.init)
      guard
        ownerPid == processIdentifier,
        let title = window[kCGWindowName as String] as? String
      else {
        continue
      }
      let normalized = title.trimmingCharacters(in: .whitespacesAndNewlines)
      if !normalized.isEmpty {
        return normalized
      }
    }
    return nil
  }
}

@main
struct InputiaInputMethodApp {
  private static var server: IMKServer?
  private static var appDelegate: InputiaApplicationDelegate?

  static func main() {
    autoreleasepool {
      if CommandLine.arguments.contains("--open-settings") {
        runSettingsOnly()
        return
      }
      if InputiaInputMethodDiagnostics.handle(arguments: CommandLine.arguments) {
        return
      }

      let bundle = Bundle.main
      let resolvedConnectionName = bundle.object(forInfoDictionaryKey: "InputMethodConnectionName") as? String
        ?? connectionName
      let resolvedBundleIdentifier = bundle.bundleIdentifier ?? fallbackBundleIdentifier
      server = IMKServer(name: resolvedConnectionName, bundleIdentifier: resolvedBundleIdentifier)
      InputiaHost.candidatePanel = InputiaCandidatePanel()
      InputiaHost.modifierMonitor = NSEvent.addGlobalMonitorForEvents(matching: .flagsChanged) { event in
        InputiaHost.activeInputController?.handleGlobalFlagsChanged(event)
      }
      NSLog("Inputia baseline IMK server started: bundle=\(resolvedBundleIdentifier), connection=\(resolvedConnectionName)")

      let app = NSApplication.shared
      let delegate = InputiaApplicationDelegate()
      appDelegate = delegate
      app.delegate = delegate
      app.setActivationPolicy(.accessory)
      app.run()
    }
  }

  private static func runSettingsOnly() {
    let app = NSApplication.shared
    let delegate = InputiaApplicationDelegate()
    delegate.terminateWhenSettingsWindowCloses = true
    appDelegate = delegate
    app.delegate = delegate
    app.setActivationPolicy(.regular)

    let controller = InputiaSettingsWindowController()
    InputiaHost.settingsWindowController = controller
    controller.window?.delegate = delegate
    controller.showWindow(nil)
    app.activate(ignoringOtherApps: true)
    app.run()
  }
}

final class InputiaInputMethodDiagnostics {
  static func handle(arguments: [String]) -> Bool {
    guard let command = arguments.dropFirst().first else {
      return false
    }

    let diagnostics = InputiaInputMethodDiagnostics()
    switch command {
    case "--self-check":
      diagnostics.selfCheck()
      return true
    case "--bridge-self-check":
      diagnostics.bridgeSelfCheck()
      return true
    case "--bridge-memory-self-check":
      diagnostics.bridgeMemorySelfCheck()
      return true
    case "--bridge-clipboard-recall-self-check":
      diagnostics.bridgeClipboardRecallSelfCheck()
      return true
    case "--bridge-clipboard-privacy-self-check":
      diagnostics.bridgeClipboardPrivacySelfCheck()
      return true
    case "--bridge-english-completion-self-check":
      diagnostics.bridgeEnglishCompletionSelfCheck()
      return true
    case "--bridge-settings-self-check":
      diagnostics.bridgeSettingsSelfCheck()
      return true
    case "--bridge-settings-reload-self-check":
      diagnostics.bridgeSettingsReloadSelfCheck()
      return true
    case "--bridge-default-chinese-self-check":
      diagnostics.bridgeDefaultChineseSelfCheck()
      return true
    case "--bridge-direct-session-self-check":
      diagnostics.bridgeDirectSessionSelfCheck()
      return true
    case "--host-shortcut-self-check", "--shortcut-self-check":
      diagnostics.hostShortcutSelfCheck()
      return true
    case "--register-input-source":
      diagnostics.register()
      return true
    case "--enable-input-source":
      diagnostics.enable()
      return true
    case "--disable-input-source":
      diagnostics.disable()
      return true
    case "--select-input-source":
      diagnostics.select()
      return true
    case "--dump-input-source":
      diagnostics.dumpInputSource(includeAllInstalled: true)
      return true
    case "--dump-enabled-input-source":
      diagnostics.dumpInputSource(includeAllInstalled: false)
      return true
    case "--dump-matching-input-sources":
      diagnostics.dumpMatchingInputSources()
      return true
    case "--dump-current-input-source":
      diagnostics.dumpCurrentInputSource()
      return true
    case "--dump-source-prefix":
      guard let prefix = arguments.dropFirst(2).first else {
        print("sourcePrefixMissing=true")
        return true
      }
      diagnostics.dumpInputSources(matchingPrefix: prefix)
      return true
    default:
      return false
    }
  }

  private func selfCheck() {
    let bundle = Bundle.main
    print("bundleIdentifier=\(bundle.bundleIdentifier ?? "unknown")")
    print(
      "connectionName=\(bundle.object(forInfoDictionaryKey: "InputMethodConnectionName") as? String ?? "unknown")"
    )

    for key in [
      "NSPrincipalClass",
      "InputMethodServerControllerClass",
      "InputMethodServerDelegateClass",
    ] {
      let className = bundle.object(forInfoDictionaryKey: key) as? String ?? ""
      print("\(key)=\(className)")
      print("classFound=\(NSClassFromString(className) != nil)")
    }
  }

  private func bridgeSelfCheck() {
    let bridge = InputiaRustBridge.temporaryForDiagnostics()
    printBridgeSelfCheck(name: "bridgeSelfCheck", outcomes: bridge.debugFullPinyinSelfCheck())
  }

  private func bridgeMemorySelfCheck() {
    let bridge = InputiaRustBridge.temporaryForDiagnostics()
    printBridgeSelfCheck(name: "bridgeMemorySelfCheck", outcomes: bridge.debugMemorySelfCheck())
  }

  private func bridgeClipboardRecallSelfCheck() {
    let bridge = InputiaRustBridge.temporaryForDiagnostics()
    let candidates = bridge.debugClipboardRecallSelfCheck()
    print("bridgeClipboardRecallSelfCheck=\(!candidates.isEmpty)")
    print("firstCandidate=\(candidates.first ?? "")")
    print("candidateCount=\(candidates.count)")
  }

  private func bridgeClipboardPrivacySelfCheck() {
    let root = URL(fileURLWithPath: NSTemporaryDirectory())
      .appendingPathComponent("InputiaClipboardPrivacySelfCheck-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
    let result = InputiaRustBridge.debugClipboardPrivacySelfCheck(
      settingsPath: root.appendingPathComponent("settings.json").path
    )
    print("bridgeClipboardPrivacySelfCheck=\(result["textedit"] == true && result["onepassword"] == false && result["unknown"] == false && result["privateWindow"] == false)")
    print("texteditAllowsClipboardRead=\(result["textedit"] == true)")
    print("onepasswordAllowsClipboardRead=\(result["onepassword"] == true)")
    print("unknownAllowsClipboardRead=\(result["unknown"] == true)")
    print("privateWindowAllowsClipboardRead=\(result["privateWindow"] == true)")
  }

  private func bridgeEnglishCompletionSelfCheck() {
    let bridge = InputiaRustBridge.temporaryForDiagnostics()
    let candidates = bridge.debugEnglishCompletionSelfCheck()
    print("bridgeEnglishCompletionSelfCheck=\(candidates.first == "Inputia")")
    print("firstCandidate=\(candidates.first ?? "")")
    print("candidateCount=\(candidates.count)")
  }

  private func bridgeSettingsSelfCheck() {
    let bridge = InputiaRustBridge.temporarySettingsForDiagnostics()
    printBridgeSelfCheck(name: "bridgeSettingsSelfCheck", outcomes: bridge.debugSettingsSelfCheck())
  }

  private func bridgeSettingsReloadSelfCheck() {
    let root = URL(fileURLWithPath: NSTemporaryDirectory())
      .appendingPathComponent("InputiaSettingsReloadSelfCheck-\(ProcessInfo.processInfo.processIdentifier)", isDirectory: true)
    let outcomes = InputiaRustBridge.debugSettingsReloadSelfCheck(
      settingsPath: root.appendingPathComponent("settings.json").path
    )
    printBridgeSelfCheck(name: "bridgeSettingsReloadSelfCheck", outcomes: outcomes)
  }

  private func bridgeDefaultChineseSelfCheck() {
    let bridge = InputiaRustBridge.makeDefault()
    printBridgeSelfCheck(name: "bridgeDefaultChineseSelfCheck", outcomes: bridge.debugDefaultChineseSelfCheck())
  }

  private func bridgeDirectSessionSelfCheck() {
    let bridge = InputiaRustBridge.temporaryDirectForDiagnostics()
    let outcome = bridge.debugCandidatePageSizeSelfCheck()
    print("bridgeDirectSessionSelfCheck=\(outcome.ok && outcome.candidates.count == 7)")
    print("candidateCount=\(outcome.candidates.count)")
    print("firstCandidate=\(outcome.candidates.first ?? "")")
  }

  private func hostShortcutSelfCheck() {
    let punctuationToggle = InputiaShortcutClassifier.isPunctuationToggle(
      keyCode: keyCodePeriod,
      charactersIgnoringModifiers: ".",
      modifiers: [.control]
    )
    let punctuationWithShift = InputiaShortcutClassifier.isPunctuationToggle(
      keyCode: keyCodePeriod,
      charactersIgnoringModifiers: ".",
      modifiers: [.control, .shift]
    )
    let punctuationWithCommand = InputiaShortcutClassifier.isPunctuationToggle(
      keyCode: keyCodePeriod,
      charactersIgnoringModifiers: ".",
      modifiers: [.control, .command]
    )
    let characterWidthToggle = InputiaShortcutClassifier.isCharacterWidthToggle(
      keyCode: keyCodeSpace,
      modifiers: [.shift]
    )
    let characterWidthWithControl = InputiaShortcutClassifier.isCharacterWidthToggle(
      keyCode: keyCodeSpace,
      modifiers: [.shift, .control]
    )
    let characterWidthPlainSpace = InputiaShortcutClassifier.isCharacterWidthToggle(
      keyCode: keyCodeSpace,
      modifiers: []
    )
    let clipboardRecall = InputiaShortcutClassifier.isClipboardRecall(
      charactersIgnoringModifiers: "v",
      modifiers: [.control, .shift]
    )
    let clipboardWithCommand = InputiaShortcutClassifier.isClipboardRecall(
      charactersIgnoringModifiers: "v",
      modifiers: [.control, .shift, .command]
    )
    let shiftInputModeArmsWhenConfigured = InputiaShortcutClassifier.shouldArmShiftInputModeToggle(
      shortcut: "shift",
      modifiers: [.shift]
    )
    let shiftInputModeRejectedWhenDisabled = InputiaShortcutClassifier.shouldArmShiftInputModeToggle(
      shortcut: "none",
      modifiers: [.shift]
    )
    let shiftInputModeReleaseTogglesWhenArmed = InputiaShortcutClassifier.isShiftInputModeToggleRelease(
      shortcut: "shift",
      hadShift: true,
      hasShift: false,
      hasBlockingModifier: false,
      armed: true
    )
    let controlSpaceInputModeTogglesWhenConfigured = InputiaShortcutClassifier.isControlSpaceInputModeToggle(
      keyCode: keyCodeSpace,
      modifiers: [.control],
      shortcut: "control_space"
    )
    let controlSpaceInputModeRejectedWhenShiftConfigured = InputiaShortcutClassifier.isControlSpaceInputModeToggle(
      keyCode: keyCodeSpace,
      modifiers: [.control],
      shortcut: "shift"
    )
    let rawCompositionOneSelectsFallback = InputiaShortcutClassifier.isDisplayedRawCompositionSelection(
      characters: "1",
      charactersIgnoringModifiers: "1",
      modifiers: [],
      hasComposing: true,
      hasCandidates: false
    )
    let rawCompositionTwoRejected = InputiaShortcutClassifier.isDisplayedRawCompositionSelection(
      characters: "2",
      charactersIgnoringModifiers: "2",
      modifiers: [],
      hasComposing: true,
      hasCandidates: false
    )
    let rawCompositionOneRejectedWhenCandidatesExist = InputiaShortcutClassifier.isDisplayedRawCompositionSelection(
      characters: "1",
      charactersIgnoringModifiers: "1",
      modifiers: [],
      hasComposing: true,
      hasCandidates: true
    )
    let rawCompositionOneRejectedWithCommand = InputiaShortcutClassifier.isDisplayedRawCompositionSelection(
      characters: "1",
      charactersIgnoringModifiers: "1",
      modifiers: [.command],
      hasComposing: true,
      hasCandidates: false
    )
    let candidateDownArrowExpandsWhenComposing = InputiaShortcutClassifier.candidateNavigation(
      keyCode: keyCodeDownArrow,
      modifiers: [],
      hasComposing: true
    ) == .expandOrNextPage
    let candidateUpArrowPagesWhenComposing = InputiaShortcutClassifier.candidateNavigation(
      keyCode: keyCodeUpArrow,
      modifiers: [],
      hasComposing: true
    ) == .previousPage
    let candidateDownArrowRejectedWithoutComposition = InputiaShortcutClassifier.candidateNavigation(
      keyCode: keyCodeDownArrow,
      modifiers: [],
      hasComposing: false
    ) == nil
    let candidateDownArrowRejectedWithCommand = InputiaShortcutClassifier.candidateNavigation(
      keyCode: keyCodeDownArrow,
      modifiers: [.command],
      hasComposing: true
    ) == nil
    let inputTextCarriageReturnIsEnter = InputiaShortcutClassifier.isInputTextEnter("\r")
    let inputTextLineFeedIsEnter = InputiaShortcutClassifier.isInputTextEnter("\n")
    let inputTextLetterIsNotEnter = InputiaShortcutClassifier.isInputTextEnter("n")
    let inputTextSpaceHandledWhenComposing = InputiaShortcutClassifier.shouldHandleInputTextSpace(
      " ",
      hasComposing: true
    )
    let inputTextSpacePassesThroughWithoutComposing = InputiaShortcutClassifier.shouldHandleInputTextSpace(
      " ",
      hasComposing: false
    )
    let securityAgentUsesSecureDirect = InputiaSecureDirectPolicy.shouldUseSecureDirectMode(
      context: InputiaAppContext(bundleId: "com.apple.SecurityAgent", windowTitle: nil)
    )
    let unknownDoesNotUseSecureDirect = !InputiaSecureDirectPolicy.shouldUseSecureDirectMode(
      context: InputiaAppContext(bundleId: "unknown", windowTitle: nil)
    )
    let regularAppDoesNotUseSecureDirect = !InputiaSecureDirectPolicy.shouldUseSecureDirectMode(
      context: InputiaAppContext(bundleId: "com.openai.chat", windowTitle: nil)
    )

    let ok = punctuationToggle
      && !punctuationWithShift
      && !punctuationWithCommand
      && characterWidthToggle
      && !characterWidthWithControl
      && !characterWidthPlainSpace
      && clipboardRecall
      && !clipboardWithCommand
      && shiftInputModeArmsWhenConfigured
      && !shiftInputModeRejectedWhenDisabled
      && shiftInputModeReleaseTogglesWhenArmed
      && controlSpaceInputModeTogglesWhenConfigured
      && !controlSpaceInputModeRejectedWhenShiftConfigured
      && rawCompositionOneSelectsFallback
      && !rawCompositionTwoRejected
      && !rawCompositionOneRejectedWhenCandidatesExist
      && !rawCompositionOneRejectedWithCommand
      && candidateDownArrowExpandsWhenComposing
      && candidateUpArrowPagesWhenComposing
      && candidateDownArrowRejectedWithoutComposition
      && candidateDownArrowRejectedWithCommand
      && inputTextCarriageReturnIsEnter
      && inputTextLineFeedIsEnter
      && !inputTextLetterIsNotEnter
      && inputTextSpaceHandledWhenComposing
      && !inputTextSpacePassesThroughWithoutComposing
      && securityAgentUsesSecureDirect
      && unknownDoesNotUseSecureDirect
      && regularAppDoesNotUseSecureDirect

    print("hostShortcutSelfCheck=\(ok)")
    print("ctrlPeriodPunctuation=\(punctuationToggle)")
    print("ctrlShiftPeriodRejected=\(!punctuationWithShift)")
    print("ctrlCommandPeriodRejected=\(!punctuationWithCommand)")
    print("shiftSpaceCharacterWidth=\(characterWidthToggle)")
    print("ctrlShiftSpaceRejected=\(!characterWidthWithControl)")
    print("plainSpaceRejected=\(!characterWidthPlainSpace)")
    print("ctrlShiftVClipboardRecall=\(clipboardRecall)")
    print("ctrlShiftCommandVRejected=\(!clipboardWithCommand)")
    print("shiftInputModeArmsWhenConfigured=\(shiftInputModeArmsWhenConfigured)")
    print("shiftInputModeRejectedWhenDisabled=\(!shiftInputModeRejectedWhenDisabled)")
    print("shiftInputModeReleaseTogglesWhenArmed=\(shiftInputModeReleaseTogglesWhenArmed)")
    print("controlSpaceInputModeTogglesWhenConfigured=\(controlSpaceInputModeTogglesWhenConfigured)")
    print("controlSpaceInputModeRejectedWhenShiftConfigured=\(!controlSpaceInputModeRejectedWhenShiftConfigured)")
    print("rawCompositionOneSelectsFallback=\(rawCompositionOneSelectsFallback)")
    print("rawCompositionTwoRejected=\(!rawCompositionTwoRejected)")
    print("rawCompositionOneRejectedWhenCandidatesExist=\(!rawCompositionOneRejectedWhenCandidatesExist)")
    print("rawCompositionOneRejectedWithCommand=\(!rawCompositionOneRejectedWithCommand)")
    print("candidateDownArrowExpandsWhenComposing=\(candidateDownArrowExpandsWhenComposing)")
    print("candidateUpArrowPagesWhenComposing=\(candidateUpArrowPagesWhenComposing)")
    print("candidateDownArrowRejectedWithoutComposition=\(candidateDownArrowRejectedWithoutComposition)")
    print("candidateDownArrowRejectedWithCommand=\(candidateDownArrowRejectedWithCommand)")
    print("inputTextCarriageReturnIsEnter=\(inputTextCarriageReturnIsEnter)")
    print("inputTextLineFeedIsEnter=\(inputTextLineFeedIsEnter)")
    print("inputTextLetterIsNotEnter=\(!inputTextLetterIsNotEnter)")
    print("inputTextSpaceHandledWhenComposing=\(inputTextSpaceHandledWhenComposing)")
    print("inputTextSpacePassesThroughWithoutComposing=\(!inputTextSpacePassesThroughWithoutComposing)")
    print("securityAgentUsesSecureDirect=\(securityAgentUsesSecureDirect)")
    print("unknownDoesNotUseSecureDirect=\(unknownDoesNotUseSecureDirect)")
    print("regularAppDoesNotUseSecureDirect=\(regularAppDoesNotUseSecureDirect)")
  }

  private func printBridgeSelfCheck(name: String, outcomes: [InputiaBridgeOutcome]) {
    let last = outcomes.last ?? .error
    print("\(name)=\(last.ok)")
    print("consumed=\(last.consumed)")
    print("mode=\(last.mode)")
    print("composing=\(last.composing)")
    print("firstCandidate=\(outcomes.first { !$0.candidates.isEmpty }?.candidates.first ?? "")")
    print("commit=\(last.commit ?? "")")
  }

  private func register() {
    let status = TISRegisterInputSource(Bundle.main.bundleURL as CFURL)
    print("registerStatus=\(status)")
  }

  private func enable() {
    let modeSources = inputModeSources(includeAllInstalled: true)
    guard !modeSources.isEmpty else {
      print("inputSourceFound=false")
      return
    }

    if let enabledSource = inputSource(includeAllInstalled: false) {
      print("enabledSourceAlreadyPresent=true")
      printSource(enabledSource)
    }

    if let parentSource = inputSource(inputSourceID: bundleIdentifier, includeAllInstalled: true) {
      let status = TISEnableInputSource(parentSource)
      print("enableParentStatus=\(status)")
      printSource(parentSource)
    }

    guard let source = primaryInputModeSource(includeAllInstalled: true) else {
      print("primaryInputModeFound=false")
      return
    }

    let status = TISEnableInputSource(source)
    print("enableModeStatus=\(status)")
    printSource(source)

    if let enabledSource = inputSource(includeAllInstalled: false) {
      print("enabledListSourceFound=true")
      printSource(enabledSource)
    } else {
      print("enabledListSourceFound=false")
    }
  }

  private func disable() {
    let modeSources = inputModeSources(includeAllInstalled: true)
    guard !modeSources.isEmpty else {
      print("inputSourceFound=false")
      return
    }

    for source in modeSources.reversed() {
      let status = TISDisableInputSource(source)
      print("disableStatus=\(status)")
      printSource(source)
    }
  }

  private func select() {
    if selectableInputModeSource(includeAllInstalled: false) == nil {
      enable()
    }
    guard let source = selectableInputModeSource(includeAllInstalled: false) else {
      print("inputSourceFoundInEnabledList=false")
      return
    }

    let status = TISSelectInputSource(source)
    print("selectStatus=\(status)")
    printSource(source)
  }

  private func dumpInputSource(includeAllInstalled: Bool) {
    guard let source = inputSource(includeAllInstalled: includeAllInstalled) else {
      print("inputSourceFound=false")
      return
    }
    printSource(source)
  }

  private func dumpCurrentInputSource() {
    let source = TISCopyCurrentKeyboardInputSource().takeRetainedValue()
    printSource(source)
  }

  private func dumpMatchingInputSources() {
    dumpInputSources(matchingPrefix: bundleIdentifier)
  }

  private func dumpInputSources(matchingPrefix prefix: String) {
    for includeAllInstalled in [false, true] {
      print("includeAllInstalled=\(includeAllInstalled)")
      guard let unmanagedSourceList = TISCreateInputSourceList(nil, includeAllInstalled) else {
        print("sourceList=false")
        continue
      }

      let matches = matchingSources(
        sourceList: unmanagedSourceList.takeRetainedValue() as! [TISInputSource],
        matchingPrefix: prefix
      )
      if matches.isEmpty {
        print("matches=0")
      }
      for source in matches {
        printSource(source)
      }
    }
  }

  private var bundleIdentifier: String {
    Bundle.main.bundleIdentifier ?? fallbackBundleIdentifier
  }

  private var inputModeIdentifiers: [String] {
    let infoDictionary = Bundle.main.infoDictionary ?? [:]
    guard
      let componentInputModeDictionary = infoDictionary["ComponentInputModeDict"] as? [String: Any],
      let inputModeList = componentInputModeDictionary["tsInputModeListKey"] as? [String: Any]
    else {
      return []
    }

    let fallbackModeIdentifiers = Array(inputModeList.keys).sorted()
    guard
      let visibleOrder = componentInputModeDictionary["tsVisibleInputModeOrderedArrayKey"] as? [String]
    else {
      return fallbackModeIdentifiers
    }

    return visibleOrder.filter { inputModeList[$0] != nil }
      + fallbackModeIdentifiers.filter { !visibleOrder.contains($0) }
  }

  private var inputSourceIdentifiers: [String] {
    [bundleIdentifier] + inputModeIdentifiers
  }

  private func inputSource(includeAllInstalled: Bool) -> TISInputSource? {
    inputModeIdentifiers.lazy
      .compactMap { self.inputModeSource(inputModeID: $0, includeAllInstalled: includeAllInstalled) }
      .first
      ?? self.inputSource(inputSourceID: bundleIdentifier, includeAllInstalled: includeAllInstalled)
  }

  private func inputSources(includeAllInstalled: Bool) -> [TISInputSource] {
    inputSourceIdentifiers.compactMap { inputSourceID in
      inputSource(inputSourceID: inputSourceID, includeAllInstalled: includeAllInstalled)
    }
  }

  private func inputModeSources(includeAllInstalled: Bool) -> [TISInputSource] {
    inputModeIdentifiers.compactMap { inputSourceID in
      inputModeSource(inputModeID: inputSourceID, includeAllInstalled: includeAllInstalled)
    }
  }

  private func primaryInputModeSource(includeAllInstalled: Bool) -> TISInputSource? {
    inputModeIdentifiers.lazy
      .compactMap { self.inputModeSource(inputModeID: $0, includeAllInstalled: includeAllInstalled) }
      .first
  }

  private func selectableInputModeSource(includeAllInstalled: Bool) -> TISInputSource? {
    inputModeSources(includeAllInstalled: includeAllInstalled)
      .first { boolProperty($0, key: kTISPropertyInputSourceIsSelectCapable) == true }
  }

  private func inputModeSource(inputModeID: String, includeAllInstalled: Bool) -> TISInputSource? {
    let properties = NSMutableDictionary()
    properties.setValue(inputModeID, forKey: kTISPropertyInputModeID as String)
    guard let unmanagedSourceList = TISCreateInputSourceList(properties, includeAllInstalled) else {
      return nil
    }

    let sourceList = unmanagedSourceList.takeRetainedValue() as! [TISInputSource]
    return sourceList.first { source in
      stringProperty(source, key: kTISPropertyInputModeID) == inputModeID
    }
  }

  private func inputSource(inputSourceID: String, includeAllInstalled: Bool) -> TISInputSource? {
    let properties = NSMutableDictionary()
    properties.setValue(inputSourceID, forKey: kTISPropertyInputSourceID as String)
    guard let unmanagedSourceList = TISCreateInputSourceList(properties, includeAllInstalled) else {
      return nil
    }

    let sourceList = unmanagedSourceList.takeRetainedValue() as! [TISInputSource]
    return sourceList.first { source in
      stringProperty(source, key: kTISPropertyInputSourceID) == inputSourceID
    }
  }

  private func matchingSources(sourceList: [TISInputSource], matchingPrefix prefix: String) -> [TISInputSource] {
    sourceList
      .filter { source in
        let bundleID = stringProperty(source, key: kTISPropertyBundleID) ?? ""
        let sourceID = stringProperty(source, key: kTISPropertyInputSourceID) ?? ""
        let modeID = stringProperty(source, key: kTISPropertyInputModeID) ?? ""
        return bundleID.hasPrefix(prefix) || sourceID.hasPrefix(prefix) || modeID.hasPrefix(prefix)
      }
      .sorted { lhs, rhs in
        let lhsID = stringProperty(lhs, key: kTISPropertyInputSourceID) ?? ""
        let rhsID = stringProperty(rhs, key: kTISPropertyInputSourceID) ?? ""
        return lhsID < rhsID
      }
  }

  private func printSource(_ source: TISInputSource) {
    print("id=\(stringProperty(source, key: kTISPropertyInputSourceID) ?? "unknown")")
    print("bundle=\(stringProperty(source, key: kTISPropertyBundleID) ?? "unknown")")
    print("mode=\(stringProperty(source, key: kTISPropertyInputModeID) ?? "unknown")")
    print("name=\(stringProperty(source, key: kTISPropertyLocalizedName) ?? "unknown")")
    print("category=\(stringProperty(source, key: kTISPropertyInputSourceCategory) ?? "unknown")")
    print("type=\(stringProperty(source, key: kTISPropertyInputSourceType) ?? "unknown")")
    print("iconURL=\(urlProperty(source, key: kTISPropertyIconImageURL) ?? "unknown")")
    print("languages=\(stringArrayProperty(source, key: kTISPropertyInputSourceLanguages).joined(separator: ","))")
    print("enabled=\(boolProperty(source, key: kTISPropertyInputSourceIsEnabled).map(String.init) ?? "unknown")")
    print(
      "enableCapable=\(boolProperty(source, key: kTISPropertyInputSourceIsEnableCapable).map(String.init) ?? "unknown")"
    )
    print(
      "selectable=\(boolProperty(source, key: kTISPropertyInputSourceIsSelectCapable).map(String.init) ?? "unknown")"
    )
    print("selected=\(boolProperty(source, key: kTISPropertyInputSourceIsSelected).map(String.init) ?? "unknown")")
  }

  private func stringProperty(_ source: TISInputSource, key: CFString!) -> String? {
    let propertyRef = TISGetInputSourceProperty(source, key)
    return unsafeBitCast(propertyRef, to: CFString?.self) as String?
  }

  private func boolProperty(_ source: TISInputSource, key: CFString!) -> Bool? {
    let propertyRef = TISGetInputSourceProperty(source, key)
    return unsafeBitCast(propertyRef, to: CFBoolean?.self).map { CFBooleanGetValue($0) }
  }

  private func urlProperty(_ source: TISInputSource, key: CFString!) -> String? {
    let propertyRef = TISGetInputSourceProperty(source, key)
    return unsafeBitCast(propertyRef, to: CFURL?.self).map { ($0 as URL).path }
  }

  private func stringArrayProperty(_ source: TISInputSource, key: CFString!) -> [String] {
    let propertyRef = TISGetInputSourceProperty(source, key)
    guard let property = unsafeBitCast(propertyRef, to: CFArray?.self) as? [Any] else {
      return []
    }
    return property.map { "\($0)" }
  }
}
