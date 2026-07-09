import Cocoa
import InputMethodKit

private struct InputiaAppContext: Equatable {
  let bundleId: String
  let windowTitle: String?
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
}

@objc(InputiaInputController)
final class InputiaInputController: IMKInputController {
  private let bridge = InputiaRustBridge.makeDefault()
  private var latestCandidates: [String] = []
  private var latestPanelCandidates: [String] = []
  private var latestComposing = ""
  private var recallCandidates: [String] = []
  private var englishCompletionPrefix = ""
  private var englishCompletionCandidates: [String] = []
  private var candidatePanelExpanded = false
  private var lastModifiers = NSEvent.ModifierFlags()
  private var shiftKeyDownWithoutOtherKey = false
  private var cachedAppContext: InputiaAppContext?
  private var cachedAppContextTime = Date.distantPast
  private var pushedAppContext: InputiaAppContext?
  private var cachedSensitiveContext: InputiaAppContext?
  private var cachedSensitiveDecision = false
  private var lastSettingsReloadCheck = Date.distantPast
  private let appContextRefreshInterval: TimeInterval = 0.75
  private let settingsReloadInterval: TimeInterval = 0.5

  override func inputText(_ string: String!, client sender: Any!) -> Bool {
    guard
      let string,
      !string.isEmpty,
      let client = sender as? IMKTextInput
    else {
      return false
    }

    if shouldPassThroughSecureDirectClient(client) {
      return false
    }

    if latestComposing.isEmpty,
      InputiaShortcutClassifier.isInputTextEnter(string) || string == " "
    {
      clearEnglishCompletion()
      return false
    }

    if shouldPassThroughSensitiveClient(client) {
      return false
    }
    updateAppContext(client: client)
    var handled = false
    for character in string {
      let outcome: InputiaBridgeOutcome
      switch InputiaInputTextRouter.action(
        for: character,
        hasComposing: !latestComposing.isEmpty
      ) {
      case .passthrough:
        return handled
      case .enter:
        let previousComposing = latestComposing
        let previousCandidates = latestCandidates
        outcome = bridge.enter()
        let shouldPassThroughNewline = InputiaHostTextPolicy.shouldPassThroughNewlineAfterRawFallbackCommit(
          previousComposing: previousComposing,
          candidates: previousCandidates,
          committedText: outcome.commit
        )
        handled = apply(outcome, client: client) || handled
        if shouldPassThroughNewline {
          return false
        }
        updateEnglishCompletionAfterCharacter(character, outcome: outcome, client: client)
        continue
      case .space:
        outcome = bridge.space()
      case .character(let routedCharacter):
        outcome = bridge.handle(character: routedCharacter)
      }
      handled = apply(outcome, client: client) || handled
      updateEnglishCompletionAfterCharacter(character, outcome: outcome, client: client)
    }
    return handled
  }

  override func handle(_ event: NSEvent!, client sender: Any!) -> Bool {
    guard
      let event,
      let client = sender as? IMKTextInput
    else {
      return false
    }

    if shouldPassThroughSecureDirectClient(client) {
      return false
    }

    if shouldPassThroughSensitiveClient(client) {
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

  override func didCommand(by aSelector: Selector!, client sender: Any!) -> Bool {
    guard
      let aSelector,
      let client = sender as? IMKTextInput
    else {
      return false
    }

    if shouldPassThroughSensitiveClient(client) {
      return false
    }
    updateAppContext(client: client)

    let selectorName = NSStringFromSelector(aSelector)
    if InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName: selectorName) {
      clearEnglishCompletion()
      return false
    }
    if InputiaHostTextPolicy.shouldPassThroughNewlineCommand(
      selectorName: selectorName,
      hasComposing: !latestComposing.isEmpty
    ) {
      clearEnglishCompletion()
      return false
    }

    switch selectorName {
    case "insertNewline:", "insertLineBreak:":
      let previousComposing = latestComposing
      let previousCandidates = latestCandidates
      let outcome = bridge.enter()
      let shouldPassThroughNewline = InputiaHostTextPolicy.shouldPassThroughNewlineAfterRawFallbackCommit(
        previousComposing: previousComposing,
        candidates: previousCandidates,
        committedText: outcome.commit
      )
      let handled = apply(outcome, client: client)
      return shouldPassThroughNewline ? false : handled
    case "deleteBackward:":
      guard !latestComposing.isEmpty else {
        return false
      }
      let outcome = bridge.backspace()
      let handled = apply(outcome, client: client)
      updateEnglishCompletionAfterBackspace(outcome: outcome, client: client)
      return handled
    case "cancelOperation:":
      guard !latestComposing.isEmpty || !englishCompletionCandidates.isEmpty else {
        return false
      }
      clearEnglishCompletion()
      return apply(bridge.escape(), client: client)
    case "insertTab:":
      return commitFirstEnglishCompletion(client: client)
    case "moveDown:":
      return handleCandidateNavigation(.expandOrNextPage, client: client)
    case "moveUp:":
      return handleCandidateNavigation(.previousPage, client: client)
    case "pageDown:":
      return handleCandidatePageDown(client: client)
    case "pageUp:":
      return handleCandidatePageUp(client: client)
    default:
      return false
    }
  }

  override func candidates(_ sender: Any!) -> [Any]! {
    InputiaHostTextPolicy.candidatesForPanel(
      composing: latestComposing,
      candidates: latestCandidates
    )
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
    if let currentClient = client(), shouldPassThroughSecureDirectClient(currentClient) {
      return
    }

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
    guard let index = latestCandidates.firstIndex(of: selected) else {
      return
    }
    _ = apply(bridge.chooseCandidate(atZeroBasedIndex: index), client: client())
  }

  override func commitComposition(_ sender: Any!) {
    guard let client = (sender as? IMKTextInput) ?? client() else {
      return
    }
    if shouldPassThroughSecureDirectClient(client) {
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
    latestPanelCandidates = []
    englishCompletionPrefix = ""
    englishCompletionCandidates = []
    candidatePanelExpanded = false
    InputiaHost.candidatePanel?.hide()
  }

  override func hidePalettes() {
    InputiaHost.candidatePanel?.hide()
    super.hidePalettes()
  }

  override func activateServer(_ sender: Any!) {
    if let client = sender as? IMKTextInput {
      if shouldPassThroughSecureDirectClient(client) {
        return
      }
      if shouldPassThroughSensitiveClient(client) {
        return
      }
      updateAppContext(client: client, forceRefresh: true)
    }
  }

  override func deactivateServer(_ sender: Any!) {
    commitComposition(sender)
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
        return apply(bridge.toggleInputMode(), client: client)
      }
    }

    lastModifiers = modifiers
    return false
  }

  private func handleKeyDown(_ event: NSEvent, client: IMKTextInput) -> Bool {
    let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
    inputiaDebugLog(
      "keyDown keyCode=\(event.keyCode) modifiers=\(modifiers.rawValue) chars=\(event.characters ?? "") charsIgnoring=\(event.charactersIgnoringModifiers ?? "")"
    )
    if InputiaShortcutClassifier.shouldPassThroughKeyDown(
      keyCode: event.keyCode,
      modifiers: modifiers
    ) {
      shiftKeyDownWithoutOtherKey = false
      return false
    }
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
      if InputiaHostTextPolicy.shouldContinueCompositionAfterCommit(
        nextComposing: outcome.composing,
        committedText: commit
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
    latestComposing = outcome.composing
    latestCandidates = outcome.candidates
    latestPanelCandidates = outcome.panelCandidates
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
        updateCandidateWindow(client: client)
        inputiaDebugLog("candidatePanelExpanded")
        return true
      }
      return handleCandidatePageDown(client: client)
    case .previousPage:
      if candidatePanelExpanded, bridge.latestOutcome.page == 0 {
        candidatePanelExpanded = false
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
    let handled = apply(bridge.pageDown(), client: client)
    return handled || !latestComposing.isEmpty
  }

  private func handleCandidatePageUp(client: IMKTextInput) -> Bool {
    guard !latestComposing.isEmpty else {
      return false
    }
    candidatePanelExpanded = true
    let handled = apply(bridge.pageUp(), client: client)
    return handled || !latestComposing.isEmpty
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
    if shouldPassThroughSecureDirectClient(client) {
      return false
    }

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
    latestPanelCandidates = candidates
    candidatePanelExpanded = false

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
    latestPanelCandidates = []
    latestComposing = ""
    candidatePanelExpanded = false
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
    latestPanelCandidates = candidates
    candidatePanelExpanded = false

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
      latestPanelCandidates = []
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
    latestPanelCandidates = []
    latestComposing = ""
    englishCompletionPrefix = ""
    englishCompletionCandidates = []
    candidatePanelExpanded = false
    _ = bridge.escape()
    InputiaHost.candidatePanel?.hide()
  }

  private func clearSecureDirectState() {
    let shouldResetBridge = !latestComposing.isEmpty || !latestCandidates.isEmpty
      || !recallCandidates.isEmpty || !englishCompletionPrefix.isEmpty
      || !englishCompletionCandidates.isEmpty || !bridge.latestOutcome.composing.isEmpty
    recallCandidates = []
    latestCandidates = []
    latestPanelCandidates = []
    latestComposing = ""
    englishCompletionPrefix = ""
    englishCompletionCandidates = []
    candidatePanelExpanded = false
    shiftKeyDownWithoutOtherKey = false
    if shouldResetBridge {
      _ = bridge.escape()
    }
    InputiaHost.candidatePanel?.hide()
  }

  private func updateCandidateWindow(client: IMKTextInput) {
    guard let panel = InputiaHost.candidatePanel else {
      return
    }
    let displayedCandidates = InputiaHostTextPolicy.candidatesForPanel(
      composing: latestComposing,
      candidates: candidatePanelExpanded && !latestPanelCandidates.isEmpty
        ? latestPanelCandidates
        : latestCandidates
    )
    if displayedCandidates.isEmpty {
      panel.hide()
      return
    }

    var inputRect = NSRect.zero
    client.attributes(forCharacterIndex: 0, lineHeightRectangle: &inputRect)
    panel.show(candidates: displayedCandidates, near: inputRect, expanded: candidatePanelExpanded)
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

  private func shouldPassThroughSensitiveClient(_ client: IMKTextInput) -> Bool {
    if shouldPassThroughSecureDirectClient(client) {
      return true
    }

    reloadSettingsIfDue(client: client)
    let context = appContext(for: client)
    let isSensitive: Bool
    if cachedSensitiveContext == context {
      isSensitive = cachedSensitiveDecision
    } else {
      isSensitive = bridge.isSensitiveApp(bundleId: context.bundleId, windowTitle: context.windowTitle)
      cachedSensitiveContext = context
      cachedSensitiveDecision = isSensitive
    }

    guard isSensitive else {
      return false
    }
    clearInputState(client: client)
    inputiaDebugLog("sensitivePassthrough bundle=\(context.bundleId) window=\(context.windowTitle ?? "")")
    if pushedAppContext != context {
      _ = bridge.setAppContext(bundleId: context.bundleId, windowTitle: context.windowTitle)
      pushedAppContext = context
    }
    return true
  }

  private func shouldPassThroughSecureDirectClient(_ client: IMKTextInput) -> Bool {
    let bundleId = client.bundleIdentifier()
    guard InputiaHostTextPolicy.isSecureDirectBundleIdentifier(bundleId) else {
      return false
    }
    clearSecureDirectState()
    cachedAppContext = nil
    cachedSensitiveContext = nil
    cachedSensitiveDecision = false
    inputiaDebugLog("secureDirectPassthrough bundle=\(bundleId ?? "unknown")")
    return true
  }

  private func appContext(for client: IMKTextInput, forceRefresh: Bool = false) -> InputiaAppContext {
    let bundleId = client.bundleIdentifier() ?? "unknown"
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

  private func reloadSettingsIfDue(client: IMKTextInput? = nil, force: Bool = false) {
    let now = Date()
    guard force || now.timeIntervalSince(lastSettingsReloadCheck) >= settingsReloadInterval else {
      return
    }
    lastSettingsReloadCheck = now
    if bridge.reloadSettingsIfNeeded() {
      clearInputState(client: client)
      cachedSensitiveContext = nil
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
    case "--disable-all-inputia-sources":
      diagnostics.disableAllInputiaSources()
      return true
    case "--select-input-source":
      diagnostics.select()
      return true
    case "--normalize-hitoolbox":
      diagnostics.normalizeHIToolbox()
      return true
    case "--clear-input-source-preferences":
      diagnostics.clearInputSourcePreferences()
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
    let result = InputiaRustBridge.debugSettingsReloadSelfCheck(
      settingsPath: root.appendingPathComponent("settings.json").path
    )
    let passed = result.outcome.ok
      && !result.noOpReloaded
      && result.noOpPreservedComposing
      && result.changedReloaded
    print("bridgeSettingsReloadSelfCheck=\(passed)")
    print("consumed=\(result.outcome.consumed)")
    print("mode=\(result.outcome.mode)")
    print("composing=\(result.outcome.composing)")
    print("firstCandidate=\(result.firstCandidate)")
    print("commit=\(result.outcome.commit ?? "")")
    print("settingsReloadNoOpReloaded=\(result.noOpReloaded)")
    print("settingsReloadNoOpPreservedComposing=\(result.noOpPreservedComposing)")
    print("settingsReloadChangedReloaded=\(result.changedReloaded)")
  }

  private func bridgeDefaultChineseSelfCheck() {
    let bridge = InputiaRustBridge.temporaryDefaultChineseForDiagnostics()
    printBridgeSelfCheck(name: "bridgeDefaultChineseSelfCheck", outcomes: bridge.debugDefaultChineseSelfCheck())
  }

  private func bridgeDirectSessionSelfCheck() {
    let bridge = InputiaRustBridge.temporaryDirectForDiagnostics()
    let outcome = bridge.debugCandidatePageSizeSelfCheck()
    let segmentedBridge = InputiaRustBridge.temporarySettingsForDiagnostics(schemaId: "double_pinyin")
    let segmented = segmentedBridge.debugSegmentedPhraseSelfCheck()
    let segmentedFirst = segmented.beforeSelection.candidates.first ?? ""
    let segmentedPhrasePreferred = segmented.beforeSelection.ok
      && segmentedFirst.count > 1
      && segmentedFirst != "你"
    let panelCandidatesBeyondCurrentPage = outcome.panelCandidates.count > outcome.candidates.count
    let singleSelectionKeepsRemaining = segmented.selectedSingle.ok
      && segmented.selectedSingle.commit == nil
      && !segmented.selectedSingle.composing.isEmpty
      && !(segmented.selectedSingle.candidates.first ?? "").isEmpty
    print(
      "bridgeDirectSessionSelfCheck=\(outcome.ok && outcome.candidates.count == 7 && panelCandidatesBeyondCurrentPage && segmentedPhrasePreferred && singleSelectionKeepsRemaining)"
    )
    print("candidateCount=\(outcome.candidates.count)")
    print("panelCandidateCount=\(outcome.panelCandidates.count)")
    print("firstCandidate=\(outcome.candidates.first ?? "")")
    print("segmentedPhraseFirstCandidate=\(segmentedFirst)")
    print("segmentedPhraseCandidateCount=\(segmented.beforeSelection.candidates.count)")
    print("segmentedSingleCandidateIndex=\(segmented.singleCandidateIndex.map { String($0) } ?? "missing")")
    print("segmentedSingleSelectionComposing=\(segmented.selectedSingle.composing)")
    print("segmentedSingleSelectionFirstCandidate=\(segmented.selectedSingle.candidates.first ?? "")")
    print("panelCandidatesBeyondCurrentPage=\(panelCandidatesBeyondCurrentPage)")
    print("segmentedPhrasePreferred=\(segmentedPhrasePreferred)")
    print("segmentedSingleSelectionKeepsRemaining=\(singleSelectionKeepsRemaining)")
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
    let scriptToggle = InputiaShortcutClassifier.isScriptToggle(
      charactersIgnoringModifiers: "s",
      modifiers: [.control, .shift],
      shortcut: "control_shift_s"
    )
    let scriptToggleWithCommand = InputiaShortcutClassifier.isScriptToggle(
      charactersIgnoringModifiers: "s",
      modifiers: [.control, .shift, .command],
      shortcut: "control_shift_s"
    )
    let scriptToggleWhenDisabled = InputiaShortcutClassifier.isScriptToggle(
      charactersIgnoringModifiers: "s",
      modifiers: [.control, .shift],
      shortcut: "none"
    )
    let commandlessShortcutNotForcedThrough = !InputiaShortcutClassifier.shouldPassThroughCommandShortcut(
      modifiers: [.control, .shift]
    )
    let commandShortcutPassThroughChecks: [(String, Bool)] = [
      (
        "commandCPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandVPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandShiftVPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command, .shift])
      ),
      (
        "commandOptionVPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command, .option])
      ),
      (
        "commandOptionShiftVPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command, .option, .shift])
      ),
      (
        "commandControlVPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command, .control])
      ),
      (
        "commandXPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandZPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandShiftZPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command, .shift])
      ),
      (
        "commandAPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandSPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandOPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandWPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandQPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandFPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandGPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandHPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandMPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandPPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandTPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandNPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandDPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandEPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandIPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandRPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandJPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandKPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandYPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandCommaPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandTabPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandSpacePassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandNumberPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandBracketPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandArrowPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandDeletePassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command])
      ),
      (
        "commandControlQPassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command, .control])
      ),
      (
        "commandShift3PassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command, .shift])
      ),
      (
        "commandShift4PassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command, .shift])
      ),
      (
        "commandShift5PassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command, .shift])
      ),
      (
        "commandOptionEscapePassThrough",
        InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.command, .option])
      ),
    ]
    let commonAppleCommandShortcutSetPassesThrough = commandShortcutPassThroughChecks.allSatisfy { $0.1 }
    let officialAppleCommandKeyDownChecks: [(String, Bool)] = [
      ("appleCommandCKeyDownPassThrough", 8, NSEvent.ModifierFlags.command),
      ("appleCommandVKeyDownPassThrough", 9, NSEvent.ModifierFlags.command),
      ("appleCommandXKeyDownPassThrough", 7, NSEvent.ModifierFlags.command),
      ("appleCommandZKeyDownPassThrough", 6, NSEvent.ModifierFlags.command),
      ("appleCommandShiftZKeyDownPassThrough", 6, [.command, .shift]),
      ("appleCommandAKeyDownPassThrough", 0, NSEvent.ModifierFlags.command),
      ("appleCommandFKeyDownPassThrough", 3, NSEvent.ModifierFlags.command),
      ("appleCommandGKeyDownPassThrough", 5, NSEvent.ModifierFlags.command),
      ("appleCommandShiftGKeyDownPassThrough", 5, [.command, .shift]),
      ("appleCommandHKeyDownPassThrough", 4, NSEvent.ModifierFlags.command),
      ("appleCommandOptionHKeyDownPassThrough", 4, [.command, .option]),
      ("appleCommandMKeyDownPassThrough", 46, NSEvent.ModifierFlags.command),
      ("appleCommandOptionMKeyDownPassThrough", 46, [.command, .option]),
      ("appleCommandNKeyDownPassThrough", 45, NSEvent.ModifierFlags.command),
      ("appleCommandOKeyDownPassThrough", 31, NSEvent.ModifierFlags.command),
      ("appleCommandPKeyDownPassThrough", 35, NSEvent.ModifierFlags.command),
      ("appleCommandSKeyDownPassThrough", 1, NSEvent.ModifierFlags.command),
      ("appleCommandWKeyDownPassThrough", 13, NSEvent.ModifierFlags.command),
      ("appleCommandQKeyDownPassThrough", 12, NSEvent.ModifierFlags.command),
      ("appleCommandBKeyDownPassThrough", 11, NSEvent.ModifierFlags.command),
      ("appleCommandIKeyDownPassThrough", 34, NSEvent.ModifierFlags.command),
      ("appleCommandUKeyDownPassThrough", 32, NSEvent.ModifierFlags.command),
      ("appleCommandKKeyDownPassThrough", 40, NSEvent.ModifierFlags.command),
      ("appleCommandLKeyDownPassThrough", 37, NSEvent.ModifierFlags.command),
      ("appleCommandCommaKeyDownPassThrough", 43, NSEvent.ModifierFlags.command),
      ("appleCommandSlashKeyDownPassThrough", 44, NSEvent.ModifierFlags.command),
      ("appleCommandShiftSlashKeyDownPassThrough", 44, [.command, .shift]),
      ("appleCommandTabKeyDownPassThrough", 48, NSEvent.ModifierFlags.command),
      ("appleCommandSpaceKeyDownPassThrough", 49, NSEvent.ModifierFlags.command),
      ("appleCommandBacktickKeyDownPassThrough", 50, NSEvent.ModifierFlags.command),
      ("appleCommandLeftBracketKeyDownPassThrough", 33, NSEvent.ModifierFlags.command),
      ("appleCommandRightBracketKeyDownPassThrough", 30, NSEvent.ModifierFlags.command),
      ("appleCommandEqualKeyDownPassThrough", 24, NSEvent.ModifierFlags.command),
      ("appleCommandMinusKeyDownPassThrough", 27, NSEvent.ModifierFlags.command),
      ("appleCommandDeleteKeyDownPassThrough", 51, NSEvent.ModifierFlags.command),
      ("appleCommandControlQKeyDownPassThrough", 12, [.command, .control]),
      ("appleCommandShift3KeyDownPassThrough", 20, [.command, .shift]),
      ("appleCommandShift4KeyDownPassThrough", 21, [.command, .shift]),
      ("appleCommandShift5KeyDownPassThrough", 23, [.command, .shift]),
      ("appleCommandOptionEscapeKeyDownPassThrough", 53, [.command, .option]),
    ].map { name, keyCode, modifiers in
      (
        name,
        InputiaShortcutClassifier.shouldPassThroughKeyDown(
          keyCode: UInt16(keyCode),
          modifiers: modifiers
        )
      )
    }
    let officialAppleCommandKeyDownSetPassesThrough = officialAppleCommandKeyDownChecks.allSatisfy { $0.1 }
    let representativeKeyCodes: [UInt16] = [
      0, 1, 2, 3, 4, 5, 6, 7, 8, 9,
      11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
      21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
      31, 32, 33, 34, 35, 37, 38, 40, 43, 44,
      45, 46, 47, 48, 49, 50, 51, 53, 76, 116,
      121, 123, 124, 125, 126,
    ]
    let commandModifierVariants: [NSEvent.ModifierFlags] = [
      .command,
      [.command, .shift],
      [.command, .option],
      [.command, .control],
      [.command, .option, .shift],
      [.command, .control, .shift],
      [.command, .control, .option],
      [.command, .control, .option, .shift],
    ]
    let anyCommandModifiedKeyPassesThrough = representativeKeyCodes.allSatisfy { keyCode in
      commandModifierVariants.allSatisfy { modifiers in
        InputiaShortcutClassifier.shouldPassThroughKeyDown(keyCode: keyCode, modifiers: modifiers)
      }
    }
    let allCommandModifierVariantsPassThrough = commandModifierVariants.allSatisfy { modifiers in
      InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: modifiers)
    }
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

    let ok = punctuationToggle
      && !punctuationWithShift
      && !punctuationWithCommand
      && characterWidthToggle
      && !characterWidthWithControl
      && !characterWidthPlainSpace
      && clipboardRecall
      && !clipboardWithCommand
      && scriptToggle
      && !scriptToggleWithCommand
      && !scriptToggleWhenDisabled
      && commandlessShortcutNotForcedThrough
      && commandShortcutPassThroughChecks.allSatisfy { $0.1 }
      && commonAppleCommandShortcutSetPassesThrough
      && officialAppleCommandKeyDownSetPassesThrough
      && anyCommandModifiedKeyPassesThrough
      && allCommandModifierVariantsPassThrough
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

    print("hostShortcutSelfCheck=\(ok)")
    print("ctrlPeriodPunctuation=\(punctuationToggle)")
    print("ctrlShiftPeriodRejected=\(!punctuationWithShift)")
    print("ctrlCommandPeriodRejected=\(!punctuationWithCommand)")
    print("shiftSpaceCharacterWidth=\(characterWidthToggle)")
    print("ctrlShiftSpaceRejected=\(!characterWidthWithControl)")
    print("plainSpaceRejected=\(!characterWidthPlainSpace)")
    print("ctrlShiftVClipboardRecall=\(clipboardRecall)")
    print("ctrlShiftCommandVRejected=\(!clipboardWithCommand)")
    print("ctrlShiftSScriptToggle=\(scriptToggle)")
    print("ctrlShiftCommandSScriptToggleRejected=\(!scriptToggleWithCommand)")
    print("scriptToggleRejectedWhenDisabled=\(!scriptToggleWhenDisabled)")
    print("commandlessShortcutNotForcedThrough=\(commandlessShortcutNotForcedThrough)")
    print("commonAppleCommandShortcutSetPassesThrough=\(commonAppleCommandShortcutSetPassesThrough)")
    print("officialAppleCommandKeyDownSetPassesThrough=\(officialAppleCommandKeyDownSetPassesThrough)")
    print("anyCommandModifiedKeyPassesThrough=\(anyCommandModifiedKeyPassesThrough)")
    print("allCommandModifierVariantsPassThrough=\(allCommandModifierVariantsPassThrough)")
    for (name, result) in commandShortcutPassThroughChecks {
      print("\(name)=\(result)")
    }
    for (name, result) in officialAppleCommandKeyDownChecks {
      print("\(name)=\(result)")
    }
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

    if let enabledSource = primaryInputModeSource(includeAllInstalled: false) {
      print("enabledSourceAlreadyPresent=true")
      printSource(enabledSource)
    } else {
      print("enabledSourceAlreadyPresent=false")
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

    if let enabledSource = primaryInputModeSource(includeAllInstalled: false) {
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

    if let parentSource = inputSource(inputSourceID: bundleIdentifier, includeAllInstalled: true) {
      let status = TISDisableInputSource(parentSource)
      print("disableParentStatus=\(status)")
      printSource(parentSource)
    }
  }

  private func disableAllInputiaSources() {
    guard let unmanagedSourceList = TISCreateInputSourceList(nil, true) else {
      print("disableAllInputiaSourcesFound=false")
      return
    }
    let sourceList = unmanagedSourceList.takeRetainedValue() as! [TISInputSource]
    let sources = sourceList
      .filter { source in
        let bundleID = stringProperty(source, key: kTISPropertyBundleID) ?? ""
        let sourceID = stringProperty(source, key: kTISPropertyInputSourceID) ?? ""
        let modeID = stringProperty(source, key: kTISPropertyInputModeID) ?? ""
        return inputiaBundlePrefixes.contains { prefix in
          bundleID.hasPrefix(prefix) || sourceID.hasPrefix(prefix) || modeID.hasPrefix(prefix)
        }
      }
      .sorted { lhs, rhs in
        let lhsID = stringProperty(lhs, key: kTISPropertyInputSourceID) ?? ""
        let rhsID = stringProperty(rhs, key: kTISPropertyInputSourceID) ?? ""
        return lhsID < rhsID
      }
    guard !sources.isEmpty else {
      print("disableAllInputiaSourcesFound=false")
      print("disableAllInputiaSourcesCount=0")
      return
    }
    print("disableAllInputiaSourcesFound=true")
    print("disableAllInputiaSourcesCount=\(sources.count)")
    for source in sources.reversed() {
      let status = TISDisableInputSource(source)
      print("disableAllStatus=\(status)")
      printSource(source)
    }
  }

  private func select() {
    enable()
    if let enabledSource = primaryInputModeSource(includeAllInstalled: false) {
      print("inputSourceFoundInEnabledList=true")
      printSource(enabledSource)
    } else {
      print("inputSourceFoundInEnabledList=false")
    }
    guard let source = primaryInputModeSource(includeAllInstalled: true) else {
      print("inputSourceFoundInInstalledList=false")
      return
    }

    let status = TISSelectInputSource(source)
    print("selectStatus=\(status)")
    printSource(source)
    let expectedID = stringProperty(source, key: kTISPropertyInputSourceID) ?? ""
    let currentSource = TISCopyCurrentKeyboardInputSource().takeRetainedValue()
    let currentID = stringProperty(currentSource, key: kTISPropertyInputSourceID) ?? ""
    print("selectExpectedID=\(expectedID)")
    print("selectCurrentID=\(currentID)")
    print("selectCurrentMatchesTarget=\(currentID == expectedID)")
  }

  private func normalizeHIToolbox() {
    let targetModeID = inputModeIdentifiers.first ?? "\(bundleIdentifier).Main"
    let enabledBefore = preferenceArray(forKey: "AppleEnabledInputSources")
    let selectedBefore = preferenceArray(forKey: "AppleSelectedInputSources")
    let historyBefore = preferenceArray(forKey: "AppleInputSourceHistory")
    let thirdPartyBefore = preferenceArray(
      forKey: "AppleEnabledThirdPartyInputSources",
      domain: inputSourcesDomain
    )

    print("hitoolboxNormalizeTargetModeID=\(targetModeID)")
    print("hitoolboxNormalizeEnabledBefore=\(enabledBefore.count)")
    print("hitoolboxNormalizeEnabledAfter=\(enabledBefore.count)")
    print("hitoolboxNormalizeSelectedBefore=\(selectedBefore.count)")
    print("hitoolboxNormalizeSelectedAfter=\(selectedBefore.count)")
    print("hitoolboxNormalizeHistoryBefore=\(historyBefore.count)")
    print("hitoolboxNormalizeHistoryAfter=\(historyBefore.count)")
    print("thirdPartyEnabledBefore=\(thirdPartyBefore.count)")
    print("thirdPartyEnabledAfter=\(thirdPartyBefore.count)")
    print("hitoolboxNormalizeSkipped=true reason=manual-hitoolbox-write-disabled")
    print("hitoolboxNormalizeRequiredAction=enable-via-system-settings-or-fix-user-preference-service")
    print("hitoolboxNormalize=true")
  }

  private func clearInputSourcePreferences() {
    let hitoolboxEnabledBefore = preferenceArray(forKey: "AppleEnabledInputSources", domain: hitoolboxDomain)
    let hitoolboxSelectedBefore = preferenceArray(forKey: "AppleSelectedInputSources", domain: hitoolboxDomain)
    let hitoolboxHistoryBefore = preferenceArray(forKey: "AppleInputSourceHistory", domain: hitoolboxDomain)
    let thirdPartyBefore = preferenceArray(
      forKey: "AppleEnabledThirdPartyInputSources",
      domain: inputSourcesDomain
    )

    let hitoolboxEnabledAfter = deduplicatedPreferenceEntries(
      hitoolboxEnabledBefore.filter { !isInputiaPreferenceEntry($0) }
    )
    let hitoolboxSelectedAfter = deduplicatedPreferenceEntries(
      hitoolboxSelectedBefore.filter { !isInputiaPreferenceEntry($0) }
    )
    let hitoolboxHistoryAfter = deduplicatedPreferenceEntries(
      hitoolboxHistoryBefore.filter { !isInputiaPreferenceEntry($0) }
    )
    let thirdPartyAfter = deduplicatedPreferenceEntries(
      thirdPartyBefore.filter { !isInputiaPreferenceEntry($0) }
    )

    print("inputSourcePreferencesClear=false")
    print("inputSourcePreferencesClearSkipped=true reason=manual-hitoolbox-write-disabled")
    print("inputSourcePreferencesClearRequiredAction=remove-inputia-via-system-settings-if-needed")
    print("hitoolboxEnabledBefore=\(hitoolboxEnabledBefore.count)")
    print("hitoolboxEnabledWouldAfter=\(hitoolboxEnabledAfter.count)")
    print("hitoolboxSelectedBefore=\(hitoolboxSelectedBefore.count)")
    print("hitoolboxSelectedWouldAfter=\(hitoolboxSelectedAfter.count)")
    print("hitoolboxHistoryBefore=\(hitoolboxHistoryBefore.count)")
    print("hitoolboxHistoryWouldAfter=\(hitoolboxHistoryAfter.count)")
    print("thirdPartyEnabledBefore=\(thirdPartyBefore.count)")
    print("thirdPartyEnabledWouldAfter=\(thirdPartyAfter.count)")
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

  private var expectedTISIconPath: String {
    Bundle.main.bundleURL
      .appendingPathComponent("Contents/Resources/inputia.pdf")
      .standardizedFileURL
      .path
  }

  private var hitoolboxDomain: CFString {
    "com.apple.HIToolbox" as CFString
  }

  private var inputSourcesDomain: CFString {
    "com.apple.inputsources" as CFString
  }

  private func inputSource(includeAllInstalled: Bool) -> TISInputSource? {
    inputModeIdentifiers.lazy
      .compactMap { self.inputSource(inputSourceID: $0, includeAllInstalled: includeAllInstalled) }
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
      inputSource(inputSourceID: inputSourceID, includeAllInstalled: includeAllInstalled)
    }
  }

  private func primaryInputModeSource(includeAllInstalled: Bool) -> TISInputSource? {
    inputModeIdentifiers.lazy
      .compactMap { self.inputSource(inputSourceID: $0, includeAllInstalled: includeAllInstalled) }
      .first
  }

  private func inputSource(inputSourceID: String, includeAllInstalled: Bool) -> TISInputSource? {
    let properties = NSMutableDictionary()
    properties.setValue(inputSourceID, forKey: kTISPropertyInputSourceID as String)
    guard let unmanagedSourceList = TISCreateInputSourceList(properties, includeAllInstalled) else {
      return nil
    }

    let sourceList = unmanagedSourceList.takeRetainedValue() as! [TISInputSource]
    let matchingSources = sourceList.filter { source in
      stringProperty(source, key: kTISPropertyInputSourceID) == inputSourceID
    }
    return matchingSources.first { source in
      urlProperty(source, key: kTISPropertyIconImageURL) == expectedTISIconPath
    } ?? matchingSources.first
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

  private func preferenceArray(forKey key: String) -> [[String: Any]] {
    preferenceArray(forKey: key, domain: hitoolboxDomain)
  }

  private func preferenceArray(forKey key: String, domain: CFString) -> [[String: Any]] {
    if let value = CFPreferencesCopyValue(
      key as CFString,
      domain,
      kCFPreferencesCurrentUser,
      kCFPreferencesAnyHost
    ) {
      let entries = preferenceEntries(from: value)
      if !entries.isEmpty {
        return entries
      }
    }
    if
      CFEqual(domain, hitoolboxDomain),
      let diskEntries = hitoolboxPreferenceArrayFromPlist(forKey: key),
      !diskEntries.isEmpty
    {
      return diskEntries
    }
    if let value = CFPreferencesCopyValue(
      key as CFString,
      domain,
      kCFPreferencesCurrentUser,
      kCFPreferencesAnyHost
    ) {
      return preferenceEntries(from: value)
    }
    guard let appValue = CFPreferencesCopyAppValue(key as CFString, domain) else {
      return []
    }
    return preferenceEntries(from: appValue)
  }

  private func hitoolboxPreferenceArrayFromPlist(forKey key: String) -> [[String: Any]]? {
    guard let preferences = readHIToolboxPreferencePlist() else {
      return nil
    }
    let entries = preferenceEntries(from: preferences[key])
    return entries.isEmpty ? nil : entries
  }

  private func preferenceEntries(from value: Any?) -> [[String: Any]] {
    guard let value else {
      return []
    }
    if let entries = value as? [[String: Any]] {
      return entries
    }
    if let array = value as? [Any] {
      return array.compactMap { preferenceEntry(from: $0) }
    }
    if let array = value as? NSArray {
      return array.compactMap { preferenceEntry(from: $0) }
    }
    return []
  }

  private func preferenceEntry(from value: Any) -> [String: Any]? {
    if let entry = value as? [String: Any] {
      return entry
    }
    if let entry = value as? [AnyHashable: Any] {
      var normalized: [String: Any] = [:]
      for (key, value) in entry {
        normalized[String(describing: key)] = value
      }
      return normalized
    }
    guard let entry = value as? NSDictionary else {
      return nil
    }

    var normalized: [String: Any] = [:]
    for (key, value) in entry {
      normalized[String(describing: key)] = value
    }
    return normalized
  }

  private func readHIToolboxPreferencePlist() -> [String: Any]? {
    guard let preferencesURL = hitoolboxPreferenceURL() else {
      return nil
    }
    guard
      FileManager.default.fileExists(atPath: preferencesURL.path),
      let data = try? Data(contentsOf: preferencesURL),
      let existing = try? PropertyListSerialization.propertyList(from: data, options: [], format: nil)
    else {
      return nil
    }
    return stringKeyedDictionary(from: existing)
  }

  private func stringKeyedDictionary(from value: Any) -> [String: Any]? {
    if let dictionary = value as? [String: Any] {
      return dictionary
    }
    if let dictionary = value as? [AnyHashable: Any] {
      var normalized: [String: Any] = [:]
      for (key, value) in dictionary {
        normalized[String(describing: key)] = value
      }
      return normalized
    }
    guard let dictionary = value as? NSDictionary else {
      return nil
    }

    var normalized: [String: Any] = [:]
    for (key, value) in dictionary {
      normalized[String(describing: key)] = value
    }
    return normalized
  }

  private func hitoolboxPreferenceURL() -> URL? {
    let home = ProcessInfo.processInfo.environment["HOME"] ?? NSHomeDirectory()
    guard !home.isEmpty else {
      return nil
    }
    return URL(fileURLWithPath: home)
      .appendingPathComponent("Library/Preferences/com.apple.HIToolbox.plist")
  }

  private func inputiaParentPreferenceEntry() -> [String: Any] {
    [
      "Bundle ID": bundleIdentifier,
      "InputSourceKind": "Keyboard Input Method",
    ]
  }

  private func inputiaModePreferenceEntry(modeID: String) -> [String: Any] {
    [
      "Bundle ID": bundleIdentifier,
      "Input Mode": modeID,
      "InputSourceKind": "Input Mode",
    ]
  }

  private func isInputiaPreferenceEntry(_ entry: [String: Any]) -> Bool {
    let bundleID = entry["Bundle ID"] as? String ?? ""
    let modeID = entry["Input Mode"] as? String ?? ""
    return inputiaBundlePrefixes.contains { prefix in
      bundleID.hasPrefix(prefix) || modeID.hasPrefix(prefix)
    }
  }

  private var inputiaBundlePrefixes: [String] {
    [
      bundleIdentifier,
      "com.inputia.inputmethod.Inputia",
      "com.iputia.inputmethod.Iputia",
      "dev.inputia.inputmethod.Inputia",
    ]
  }

  private func deduplicatedPreferenceEntries(_ entries: [[String: Any]]) -> [[String: Any]] {
    var seen = Set<String>()
    var result: [[String: Any]] = []
    for entry in entries {
      let key = preferenceEntryKey(entry)
      if seen.insert(key).inserted {
        result.append(entry)
      }
    }
    return result
  }

  private func preferenceEntryKey(_ entry: [String: Any]) -> String {
    let kind = entry["InputSourceKind"] as? String ?? ""
    let bundleID = entry["Bundle ID"] as? String ?? ""
    let modeID = entry["Input Mode"] as? String ?? ""
    let layoutName = entry["KeyboardLayout Name"] as? String ?? ""
    let layoutID = entry["KeyboardLayout ID"].map { "\($0)" } ?? ""
    return [kind, bundleID, modeID, layoutName, layoutID].joined(separator: "|")
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
