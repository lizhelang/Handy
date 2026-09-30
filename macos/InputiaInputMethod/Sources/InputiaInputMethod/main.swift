import Cocoa
import InputMethodKit
import Carbon
import OSLog

// 只记录手势阶段和布尔状态，不记录普通键值、正文、应用名或窗口标题。
private let shiftDiagnostic = Logger(subsystem: "com.inputia.shift", category: "gesture")

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
  var candidateID: String? = nil
  var originalRank: Int? = nil
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

  func applicationWillTerminate(_ notification: Notification) {
    #if INPUTIA_PAIRED_BUILD
    InputiaPermissionLifecycle.shared.stop()
    InputiaVoiceInputLauncher.invalidatePermissionWork()
    #endif
    InputiaHost.removeGlobalMonitors()
  }

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
  static let inputControllers = NSHashTable<InputiaInputController>.weakObjects()
  static func removeGlobalMonitors() {}
  static func installGlobalMonitors() {}

}

@objc(InputiaInputController)
final class InputiaInputController: IMKInputController {
  #if INPUTIA_PAIRED_BUILD
  private let personalization = InputiaPersonalization()
  private var personalCandidates: [InputiaPersonalCandidate] = []
  private var personalPredictions: [InputiaPersonalPrediction] = []
  private var personalCode = ""
  private var personalRefreshGeneration: UInt64 = 0
  private var personalExpectedSelection: NSRange?
  private var personalInputDeferral = InputiaPersonalInputDeferral<NSEvent>()
  private var personalPredictionPending: Bool { personalInputDeferral.isPending }
  private struct PersonalDeferredOrigin {
    let token: UInt64
    let origin: InputiaVoiceTargetSnapshot.Snapshot
    let client: IMKTextInput
    let activation: UInt64
    let generation: UInt64
    let epoch: UInt64
    let schemaID: String
    let code: String
    let selection: NSRange
    let marked: NSRange
    let managedField: String?
  }
  private var personalDeferredOrigin: PersonalDeferredOrigin?
  private var personalRecoveryToken: UInt64?
  private let personalBoundaryWait = InputiaPersonalBoundaryWait()
  private var personalBoundaryProof: InputiaTargetBridgeReply?
  private var personalReplayKeepsTypedOrigin = false
  private var personalEscapeConsumed = false
  private var personalOrderLockedCode: String?
  private var personalPhrase = InputiaPersonalPhraseAssembly()
  private struct PersonalSelection {
    let target: InputiaVoiceTarget
    let code: String
    let composing: String
    let schemaID: String
    let candidate: InputiaPersonalCandidate
    let explicit: Bool
  }
  private struct PersonalUndo {
    let receipts: [InputiaPersonalization.Receipt]
    let nativeLearning: Bool
    let target: InputiaVoiceTarget
    let inserted: String
    let start: Int
    let after: NSRange
    let time: TimeInterval
    let client: ObjectIdentifier
  }
  private var pendingPersonalSelection: PersonalSelection?
  private var personalUndo: PersonalUndo?
  private var permissionEpoch: UInt64 = UInt64.max
  private var typedEventOrigin: InputiaVoiceTargetSnapshot.Snapshot?
  private var typedCompositionOrigin: InputiaVoiceTargetSnapshot.Snapshot?
  private var typedRetainedOrigins: [String: TimeInterval] = [:]
  private var voicePermissionEpochs: [String: UInt64] = [:]
  func removeRetiredVoiceTarget(_ id: String) {
    memoryTargetFields.removeValue(forKey: id)
    voiceTargetSnapshots.removeValue(forKey: id)
    voicePermissionEpochs.removeValue(forKey: id)
    if shortcutPreparedSnapshot?.targetID == id {
      shortcutPreparedSnapshot = nil; sharedTargetReady = false; sharedPreparedClientIdentity = nil
    }
  }
  private func discardPreparedVoiceTarget() {
    let old = shortcutPreparedSnapshot
    shortcutPreparedSnapshot = nil; sharedTargetReady = false; sharedPreparedClientIdentity = nil
    if let old, voiceTargetSnapshots[old.targetID] == nil,
      typedEventOrigin?.targetID != old.targetID, typedCompositionOrigin?.targetID != old.targetID,
      typedRetainedOrigins[old.targetID] == nil {
      InputiaVoiceInputLauncher.releaseTarget(old.targetID)
    }
  }
  func synchronizePermissionEpoch() {
    let current = InputiaPermissionLifecycle.shared.epoch
    guard permissionEpoch != current else { return }
    permissionEpoch = current
    _ = bridge.clearManagedMemory()
    clearManagedMemoryDisplay()
    discardTypedCompositionOrigin()
    voiceActivationGeneration &+= 1
    for snapshot in voiceTargetSnapshots.values { InputiaVoiceInputLauncher.releaseTarget(snapshot.targetID) }
    if let snapshot = shortcutPreparedSnapshot { InputiaVoiceInputLauncher.releaseTarget(snapshot.targetID) }
    voiceTargetSnapshots.removeAll()
    voicePermissionEpochs.removeAll()
    attemptedVoiceOutputOperations.removeAll()
    shortcutPreparedSnapshot = nil
    voicePreCaptureRetryDeadline = nil
    sharedTargetReady = false
    sharedPreparedClientIdentity = nil
    sharedEnglishSelection.cancel(); sharedChineseSelection.cancel()
    clearSharedChineseCandidates()
    sharedEnglishRefreshQueued = false
    InputiaSharedTermsMemory.shared.clear()
    cachedAppContext = nil; pushedAppContext = nil

  }
  private let voiceControllerID = UUID().uuidString
  private var voiceActivationGeneration: UInt64 = 0
  private var voiceStatus = ""
  private var unifiedMenuSnapshot: InputiaMenuReply?
  private var voiceTargetCaptureNotice: String?
  private var voiceTargetPreparationFailure = "unknown"
  private var voiceTargetSnapshots: [String: InputiaVoiceTargetSnapshot.Snapshot] = [:]
  private var attemptedVoiceOutputOperations = Set<String>()
  private var shortcutPreparedSnapshot: InputiaVoiceTargetSnapshot.Snapshot?
  private var voicePreCaptureRetryDeadline: TimeInterval?
  private var shortcutReadinessReason = ""
  private var sharedEnglishCandidates: [String: String] = [:]
  private var sharedTargetReady = false
  private var sharedPreparedClientIdentity: ObjectIdentifier?
  private var sharedEnglishRefreshQueued = false
  private var sharedEnglishSelection = InputiaSharedEnglishSelectionState()
  private var sharedChineseSelection = InputiaSharedEnglishSelectionState()
  private var hotwordOverlay: (code: String, words: [String], base: [String], snapshot: InputiaExplicitHotwords.Snapshot, context: InputiaExplicitSelectionContext)?
  private var explicitEnglishCandidates: [String: (snapshot: InputiaExplicitHotwords.Snapshot, context: InputiaExplicitSelectionContext)] = [:]
  private struct MemoryPreparedCommit {
    let target: InputiaVoiceTarget
    let policy: InputiaMemoryPolicy
    let request: InputiaMemoryFixedRequest
    let permit: InputiaMemoryPermit
    let started: TimeInterval
    let client: ObjectIdentifier
    let activation: UInt64
  }
  private var memoryPreparedCommit: MemoryPreparedCommit?
  private var memoryCommitExpiry: DispatchWorkItem?
  private var memoryViewGeneration: UInt64 = 1
  private var memoryFieldID: String?
  private var wordSpanPreparation: UUID?
  private var wordSpanObservation = UUID()
  private lazy var wordSpan: InputiaWordSpan = InputiaWordSpan(send: { command, policy, completion in
    InputiaVoiceBridge.shared.wordSpan(command, policy: policy, completion: completion)
  }, current: { [weak self] context, caret in
    self?.wordSpanScopeCurrent(context, caret: caret) == true
  }, ended: { [weak self] context in
    InputiaVoiceInputLauncher.releaseTarget(context.target.target_id)
    guard let self else { return }
    self.removeRetiredVoiceTarget(context.target.target_id)
    if self.wordSpan.coverageReason == "sealed", let client = self.client() {
      self.prepareWordSpan(client: client)
    }
  })
  private var memoryTargetFields: [String: String] = [:]
  private var memorySelectionIntent: UUID?
  private var memoryDisplayedTargets: [String: InputiaVoiceTargetSnapshot.Snapshot] = [:]
  private var memoryDisplayedComposing: [String: String] = [:]
  private var sharedChineseOrder: (order: InputiaSharedCandidateOrder, candidates: [String], identity: String, target: InputiaVoiceTarget)?
  #endif
  private let bridge = InputiaRustBridge.makeDefault()
  private var latestCandidates: [String] = []
  private var localCompositionGeneration: UInt64 = 0
  private var localSelectionGeneration: UInt64 = 0
  private var targetCapturePending = false
  private var latestComposing = "" {
    didSet { if oldValue != latestComposing { localCompositionGeneration &+= 1 } }
  }
  private var expandedCandidates: [String] = []
  private var expandedCandidateEntries: [InputiaExpandedCandidateEntry] = []
  private var expandedActiveRowIndex = 0
  private var recallCandidates: [String] = []
  private var englishCompletionPrefix = "" {
    didSet {
      #if INPUTIA_PAIRED_BUILD
      if oldValue != englishCompletionPrefix { sharedEnglishSelection.cancel() }
      #endif
    }
  }
  private var englishCompletionCandidates: [String] = []
  private var englishCompletionRect = NSRect.zero
  private var chineseCandidateRect = NSRect.zero
  private var shiftEnglishComposition = ""
  private var candidatePanelExpanded = false
  private var shiftInputModeGesture = InputiaShortcutClassifier.ShiftInputModeGestureState()
  private weak var gestureInputClient: AnyObject?
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

    #if INPUTIA_PAIRED_BUILD
    let selectionKey = event.type == .keyDown && (event.keyCode == keyCodeSpace || event.keyCode == keyCodeReturn
      || event.keyCode == keyCodeKeypadEnter || event.keyCode == keyCodeTab
      || event.charactersIgnoringModifiers.flatMap(Int.init).map { (1...9).contains($0) } == true)
      && event.modifierFlags.intersection([.command, .control, .option]).isEmpty
    let changesInput = InputiaMemoryInputTransition.retiresPending(keyDown: event.type == .keyDown,
      modeBoundary: event.type == .flagsChanged && [56, 60].contains(event.keyCode))
    synchronizePermissionEpoch()
    #endif
    if shouldUseSecureDirectMode(client) {
      #if INPUTIA_PAIRED_BUILD
      retireWordSpan(reason: "secure_input")
      discardTypedCompositionOrigin()
      #endif
      return false
    }
    #if INPUTIA_PAIRED_BUILD
    if let deferred = deferPersonalInput(event, client: client) { return deferred }
    if event.type == .keyDown && wordSpanEdit(event, client: client) == nil { retireWordSpan(reason: "special_key_or_composition") }
    if changesInput {
      memorySelectionIntent = nil
      if !selectionKey { retireMemoryCommit() }
      memoryViewGeneration &+= 1
      bridge.cancelManagedMemoryRequests()
    }
    if event.type == .keyDown {
      if isPersonalRejectionShortcut(event), rejectPersonalCandidate(client: client) { return true }
      observePersonalKey(event, client: client)
      // 既有快捷键预捕获先于本次插入；不拿文本发送时的新字段为旧字背书。
      typedEventOrigin = shortcutPreparedSnapshot
      let boundary = !isPersonalCandidateNavigation(event) && !isPersonalCompositionBackspace(event) && ([UInt16(51), 53, 115, 116, 117, 119, 121, 123, 124, 125, 126].contains(event.keyCode)
        || !event.modifierFlags.intersection([.command, .control, .option]).isEmpty)
      refreshTypedCompositionOrigin(client: client, boundary: boundary && !personalReplayKeepsTypedOrigin)
      if boundary { MainActor.assumeIsolated { InputiaTypedCapture.shared.resetSegment() } }
      localSelectionGeneration &+= 1
      discardPreparedVoiceTarget()
    }
    #endif
    defer {
      #if INPUTIA_PAIRED_BUILD
      if let origin = typedEventOrigin {
        typedEventOrigin = nil
        releaseTypedOriginIfUnowned(origin.targetID)
      }
      #endif
    }
    cancelShiftGestureIfClientChanged(client)
    updateAppContext(client: client)

    switch event.type {
    case .flagsChanged:
      return handleFlagsChanged(event, client: client)
    case .keyDown:
      #if INPUTIA_PAIRED_BUILD
      return handleWordSpanKeyDown(event, client: client)
      #else
      return handleKeyDown(event, client: client)
      #endif
    case .keyUp:
      shiftInputModeGesture.observeLocalKeyUp(keyCode: event.keyCode)
      if [56, 60].contains(event.keyCode),
        shiftInputModeGesture.observePhysicalShiftKeyUp(
          shortcut: bridge.inputModeToggleShortcut(),
          modifiers: event.modifierFlags.intersection(.deviceIndependentFlagsMask)
        ) {
        return toggleInputModeFromShift(client: client, source: "physical-keyup")
      }
      // 仅更新手势状态，不消费宿主的按键松开，也不送入 Rime。
      return false
    default:
      return false
    }
  }

  override func recognizedEvents(_ sender: Any!) -> Int {
    Int(NSEvent.EventTypeMask(arrayLiteral: .keyDown, .keyUp, .flagsChanged).rawValue)
  }

  override func candidates(_ sender: Any!) -> [Any]! {
    // Inputia draws its own compact candidate popup. Returning candidate data
    // here makes InputMethodKit show the system IMKCandidates window as well,
    // which follows the user's accent color and can grow into an oversized bar.
    []
  }

  override func menu() -> NSMenu! {
    #if INPUTIA_PAIRED_BUILD
    return unifiedProductMenu()
    #else
    let voiceInput = NSMenuItem(title: "语音输入", action: #selector(toggleVoiceInput), keyEquivalent: "")
    voiceInput.target = self
    #if INPUTIA_PAIRED_BUILD
    // 只由IMK按键路径消费；不再注册菜单keyEquivalent，避免一次按键触发两次。
    voiceInput.title = voiceStatus.isEmpty ? "语音输入（⌃⌥⇧V）" : "语音输入（⌃⌥⇧V）：\(voiceStatus)"
    #endif

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
    #endif
  }

  #if INPUTIA_PAIRED_BUILD
  private func unifiedProductMenu() -> NSMenu {
    let menu = NSMenu(title: "Inputia")
    menu.autoenablesItems = false
    let voice = NSMenuItem(title: "开始 / 停止语音", action: #selector(toggleVoiceInput), keyEquivalent: "")
    voice.target = self
    menu.addItem(voice)
    if !voiceStatus.isEmpty {
      let status = NSMenuItem(title: String(voiceStatus.prefix(32)), action: nil, keyEquivalent: "")
      status.isEnabled = false
      menu.addItem(status)
    }
    func add(_ title: String, _ kind: String, modelID: String? = nil, to targetMenu: NSMenu? = nil) -> NSMenuItem {
      let item = NSMenuItem(title: title, action: #selector(unifiedMenuAction(_:)), keyEquivalent: "")
      item.target = self
      item.representedObject = ["kind": kind, "model_id": modelID ?? ""]
      (targetMenu ?? menu).addItem(item)
      return item
    }
    _ = add("复制最新转写", "copy_latest")
    let reject = NSMenuItem(title: "本语境不推荐当前候选（⌃⌫）", action: #selector(rejectPersonalCandidateFromMenu), keyEquivalent: "")
    reject.target = self
    reject.isEnabled = personalization.allowed && !personalCandidates.isEmpty && !candidatePanelExpanded && hotwordOverlay == nil
    menu.addItem(reject)
    let history = add(InputiaShortcutClassifier.clipboardHistoryMenuTitle(
      shortcut: unifiedMenuSnapshot?.clipboard_hotkey,
      enabled: unifiedMenuSnapshot?.clipboard_hotkey_enabled
    ), "history")
    menu.addItem(.separator())
    let models = NSMenu(title: "语音模型")
    models.autoenablesItems = false
    let modelRoot = NSMenuItem(title: "语音模型", action: nil, keyEquivalent: "")
    modelRoot.submenu = models
    menu.addItem(modelRoot)
    func renderModels(_ snapshot: InputiaMenuReply?) {
      models.removeAllItems()
      for model in snapshot?.models ?? [] {
        let item = add(model.name, "select_model", modelID: model.id, to: models)
        item.state = model.id == snapshot?.selected_model ? .on : .off
        item.isEnabled = model.available && snapshot?.busy == false
      }
      if models.items.isEmpty {
        let status = NSMenuItem(title: "在控制中心查看模型", action: nil, keyEquivalent: "")
        status.isEnabled = false
        models.addItem(status)
      }
    }
    renderModels(unifiedMenuSnapshot)
    let unload = add("卸载当前模型", "unload_model")
    unload.isEnabled = unifiedMenuSnapshot?.busy == false
    menu.addItem(.separator())
    _ = add("Inputia 设置…", "settings")
    _ = add("检查更新…", "check_updates")
    menu.addItem(.separator())
    let service = add("正在检查语音服务…", "quit_service")
    service.isEnabled = false
    InputiaVoiceInputLauncher.refreshServiceMenu { [weak self] snapshot, serviceState in
      guard let self else { return }
      self.unifiedMenuSnapshot = snapshot
      service.title = serviceState.title
      service.isEnabled = serviceState.action != nil
      service.representedObject = ["kind": serviceState.action ?? "", "model_id": ""]
      history.title = InputiaShortcutClassifier.clipboardHistoryMenuTitle(
        shortcut: snapshot?.clipboard_hotkey,
        enabled: snapshot?.clipboard_hotkey_enabled
      )
      renderModels(snapshot)
      unload.isEnabled = snapshot?.busy == false
    }
    return menu
  }

  @objc private func unifiedMenuAction(_ sender: Any?) {
    guard let values = InputiaHostTextPolicy.serviceMenuPayload(from: sender), let kind = values["kind"] else {
      NSLog("inputia_menu_command_rejected reason=invalid_sender")
      return
    }
    guard ["copy_latest", "history", "settings", "check_updates", "unload_model", "select_model", "quit_service", "start_service"].contains(kind) else { return }
    // 只记录固定动作名，绝不记录剪贴正文、模型路径或客户端字典。
    NSLog("inputia_menu_command_queued action=\(kind)")
    if kind == "start_service" {
      InputiaVoiceInputLauncher.startUnifiedService { [weak self] started in
        self?.unifiedMenuSnapshot = nil
        self?.voiceStatus = started ? "已请求在后台打开语音服务" : "打开语音服务未确认；未自动重试"
      }
      return
    }
    let modelID = values["model_id"].flatMap { $0.isEmpty ? nil : $0 }
    InputiaVoiceInputLauncher.menuAction(kind: kind, modelID: modelID) { [weak self] reply in
      self?.unifiedMenuSnapshot = reply
      self?.voiceStatus = reply == nil ? "操作未确认；未自动重试" : ""
    }
  }
  #endif

  @objc private func toggleVoiceInput() {
    #if INPUTIA_PAIRED_BUILD
    startUnifiedVoice(client: client())
    return
    #elseif INPUTIA_UNIFIED_CANDIDATE
    showHostAlert(title: "候选语音尚未配对", message: "此候选没有配对构建材料，不会启动或切换日常 Inputia。")
    #else
    switch InputiaVoiceInputLauncher.triggerVoiceInput() {
    case .started:
      return
    case .missing:
      showHostAlert(
        title: "无法启动语音输入",
        message: "没有找到 Inputia 语音服务。请先安装或启动 Inputia，再从菜单触发语音输入。"
      )
    case .failed(let message):
      showHostAlert(title: "无法启动语音输入", message: message)
    }
    #endif
  }

  @objc private func syncHandyMemory() {
    #if INPUTIA_PAIRED_BUILD
    synchronizePermissionEpoch()
    guard InputiaPermissionLifecycle.shared.isReady else { return }
    #endif
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
    let previousPolicy = app.activationPolicy()
    defer { app.setActivationPolicy(previousPolicy) }
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
    #if INPUTIA_PAIRED_BUILD
    retireWordSpan(reason: "candidate_selection")
    synchronizePermissionEpoch()
    #endif
    guard let selected = candidateString?.string else {
      return
    }
    #if INPUTIA_PAIRED_BUILD
    if hotwordOverlay != nil, !candidatePanelExpanded,
      let index = latestCandidates.firstIndex(of: selected), let client = client() {
      _ = selectHotwordOverlay(displayed: index, client: client); return
    }
    if let prediction = personalPredictions.first(where: { $0.text == selected }), let client = client() {
      _ = acceptPersonalPrediction(prediction, client: client)
      return
    }
    if let candidate = personalCandidates.first(where: { $0.text == selected }), let client = client() {
      _ = choosePersonal(candidate, explicit: true, client: client)
      return
    }
    #endif
    #if INPUTIA_PAIRED_BUILD
    if sharedChineseOrder != nil, !candidatePanelExpanded,
      let index = latestCandidates.firstIndex(of: selected), let client = client() {
      _ = enqueueSharedChineseSelection(displayed: index, client: client)
      return
    }
    #endif
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
    if let client = client() {
      _ = chooseNativeWithLearning(index: index, explicit: true, client: client)
    }
  }

  override func commitComposition(_ sender: Any!) {
    #if INPUTIA_PAIRED_BUILD
    retireWordSpan(reason: "external_commit_boundary")
    synchronizePermissionEpoch()
    #endif
    guard let client = (sender as? IMKTextInput) ?? client() else {
      return
    }
    #if INPUTIA_PAIRED_BUILD
    if let pending = personalDeferredOrigin { finishDeferredPersonalInput(pending.token) }
    #endif
    let context = appContext(for: client)
    if IsSecureEventInputEnabled() || InputiaSecureDirectPolicy.shouldUseSecureDirectMode(context: context) {
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
    InputiaHost.inputControllers.add(self)
    #if INPUTIA_PAIRED_BUILD
    bridge.memoryDidExpire = { [weak self] in self?.clearManagedMemoryDisplay() }
    _ = bridge.clearManagedMemory()
    clearManagedMemoryDisplay()
    #endif
    #if INPUTIA_PAIRED_BUILD
    discardTypedCompositionOrigin()
    personalization.changed = { [weak self] in self?.clearPersonalDisplay() }
    personalization.policyChanged = { [weak self] in
      guard let self, InputiaHost.activeInputController === self, let client = self.client() else { return }
      self.schedulePersonalization(client: client)
    }
    personalization.start()
    MainActor.assumeIsolated {
      InputiaTypedCapture.shared.invalidOrigin = { [weak self] id in
        guard self?.typedCompositionOrigin?.targetID == id else { return }
        self?.discardTypedCompositionOrigin()
      }
      InputiaTypedCapture.shared.activate()
    }
    synchronizePermissionEpoch()
    InputiaVoiceInputLauncher.ensureUnifiedServiceReady()
    sharedTargetReady = false
    InputiaSharedTermsMemory.shared.clear()
    voicePreCaptureRetryDeadline = nil
    voiceActivationGeneration &+= 1
    #endif
    resetShiftInputModeSession(reason: "activate")
    InputiaHost.activeInputController = self
    InputiaHost.candidatePanel?.settingsDidApply = { [weak self] snapshot in
      guard let self, InputiaHost.activeInputController === self else { return }
      self.bridge.candidateDisplaySettingsApplied(snapshot)
    }
    if let client = sender as? IMKTextInput {
      if shouldUseSecureDirectMode(client) {
        return
      }
      updateAppContext(client: client, forceRefresh: true)
      #if INPUTIA_PAIRED_BUILD
      prepareWordSpan(client: client)
      #endif
    }
    #if INPUTIA_PAIRED_BUILD
    refreshExplicitHotwords()
    #endif
  }

  override func deactivateServer(_ sender: Any!) {
    #if INPUTIA_PAIRED_BUILD
    _ = bridge.clearManagedMemory()
    clearManagedMemoryDisplay()
    clearHotwordOverlay()
    explicitEnglishCandidates = [:]
    personalization.stop()
    MainActor.assumeIsolated { InputiaTypedCapture.shared.deactivate() }
    discardTypedCompositionOrigin()
    discardPreparedVoiceTarget()
    sharedTargetReady = false
    InputiaSharedTermsMemory.shared.clear()
    voicePreCaptureRetryDeadline = nil
    voiceActivationGeneration &+= 1
    #endif
    resetShiftInputModeSession(reason: "deactivate")
    commitComposition(sender)
    if InputiaHost.activeInputController === self {
      InputiaHost.candidatePanel?.settingsDidApply = nil
      InputiaHost.activeInputController = nil
    }
  }

  private func handleFlagsChanged(_ event: NSEvent, client: IMKTextInput) -> Bool {
    reloadSettingsIfDue(client: client)
    let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
    let shortcut = bridge.inputModeToggleShortcut()
    shiftDiagnostic.notice("phase=local-flags shift=\(modifiers.contains(.shift)) armed=\(self.shiftInputModeGesture.isArmedForDebug) held=\(self.shiftInputModeGesture.hasHeldKeysForDebug) baseline=\(self.shiftInputModeGesture.hasShiftBaselineForDebug) configured=\(shortcut == "shift") blocked=\(!modifiers.intersection([.control, .option, .command]).isEmpty)")

    inputiaDebugLog(
      "flagsChanged keyCode=\(event.keyCode) current=\(modifiers.rawValue) shortcut=\(shortcut) armed=\(shiftInputModeGesture.isArmedForDebug)"
    )

    if shiftInputModeGesture.observeInputMethodFlagsChanged(
      shortcut: shortcut,
      modifiers: modifiers
    ) == .toggle {
      return toggleInputModeFromShift(client: client, source: "local")
    }

    return false
  }

  private func commitPendingChineseAsEnglish(client: IMKTextInput) {
    let raw = bridge.latestOutcome.composing
    guard !raw.isEmpty else { return }
    let outcome = bridge.escape()
    _ = apply(outcome, client: client)
    client.insertText(raw, replacementRange: emptyReplacementRange)
    inputiaDebugLog("shiftCommitChinesePinyinAsEnglish length=\(raw.count)")
  }

  private func toggleInputModeFromShift(client: IMKTextInput?, source: String) -> Bool {
    guard let client else {
      inputiaDebugLog("shiftToggleRejected source=\(source) reason=missing-client")
      return true
    }
    // 只有确认是独立 Shift 手势时，才提交未完成的拼音；组合键保持原模式。
    if bridge.latestOutcome.mode == "Chinese", !bridge.latestOutcome.composing.isEmpty {
      commitPendingChineseAsEnglish(client: client)
    }
    clearEnglishCompletion()
    shiftDiagnostic.notice("phase=toggle source=\(source, privacy: .public)")
    inputiaDebugLog("shiftToggle source=\(source)")
    let handled = apply(bridge.toggleInputMode(), client: client)
    #if INPUTIA_PAIRED_BUILD
    if bridge.latestOutcome.mode == "English" { prepareWordSpan(client: client) }
    #endif
    return handled
  }

  private func cancelShiftInputModeGesture(reason: String) {
    shiftInputModeGesture.cancelPendingGesture()
    inputiaDebugLog("shiftGestureCancelled reason=\(reason)")
  }

  private func resetShiftInputModeSession(reason: String) {
    shiftDiagnostic.notice("phase=session-reset reason=\(reason, privacy: .public) shift=\(NSEvent.modifierFlags.contains(.shift))")
    // 事件连续性已断开：清理可能漏收 keyUp 的普通键，仍按住的 Shift 不获得切换资格。
    shiftInputModeGesture.resetSession(
      modifiers: NSEvent.modifierFlags.intersection(.deviceIndependentFlagsMask)
    )
    inputiaDebugLog("shiftGestureSessionReset reason=\(reason)")
  }

  private func cancelShiftGestureIfClientChanged(_ client: IMKTextInput) {
    let currentClient = client as AnyObject
    if let gestureInputClient, gestureInputClient !== currentClient {
      resetShiftInputModeSession(reason: "clientChanged")
    }
    gestureInputClient = currentClient
  }

  private func handleKeyDown(_ event: NSEvent, client: IMKTextInput) -> Bool {
    let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
    let deferredShiftToggle = InputiaShortcutClassifier.shouldConsumeDeferredShiftToggle(
      armed: shiftInputModeGesture.isArmedForDebug,
      modifiers: modifiers
    )
    shiftInputModeGesture.observeLocalKeyDown(keyCode: event.keyCode, modifiers: modifiers)
    inputiaDebugLog(
      "keyDown modifiers=\(modifiers.rawValue)"
    )
    if deferredShiftToggle {
      // Some IMK clients omit the Shift flagsChanged/keyUp release. Consume
      // the pending tap on the next ordinary key, then let that same key pass
      // through the newly selected English mode.
      cancelShiftInputModeGesture(reason: "deferredReleaseBeforeKey")
      _ = toggleInputModeFromShift(client: client, source: "deferred-keydown")
    }
    if isScriptToggleShortcut(event, modifiers: modifiers) {
      cancelShiftInputModeGesture(reason: "scriptToggle")
      clearEnglishCompletion()
      clearInputState(client: client)
      return bridge.toggleChineseScriptPreference()
    }
    if isClipboardRecallShortcut(event, modifiers: modifiers) {
      cancelShiftInputModeGesture(reason: "clipboardRecall")
      return showClipboardRecall(client: client)
    }
    if !recallCandidates.isEmpty {
      if handleRecallKeyDown(event, client: client) {
        return true
      }
    }
    if isPunctuationToggleShortcut(event, modifiers: modifiers) {
      cancelShiftInputModeGesture(reason: "punctuationToggle")
      clearEnglishCompletion()
      return apply(bridge.togglePunctuationPreference(), client: client)
    }
    if isCharacterWidthToggleShortcut(event, modifiers: modifiers) {
      cancelShiftInputModeGesture(reason: "characterWidthToggle")
      clearEnglishCompletion()
      return apply(bridge.toggleCharacterWidthPreference(), client: client)
    }
    if isInputModeToggleShortcut(event, modifiers: modifiers) {
      cancelShiftInputModeGesture(reason: "controlSpaceToggle")
      clearEnglishCompletion()
      let handled = apply(bridge.toggleInputMode(), client: client)
      #if INPUTIA_PAIRED_BUILD
      if bridge.latestOutcome.mode == "English" { prepareWordSpan(client: client) }
      #endif
      return handled
    }
    if bridge.latestOutcome.mode == "Chinese",
      InputiaShortcutClassifier.isShiftEnglishCompositionCharacter(
        characters: event.characters,
        charactersIgnoringModifiers: event.charactersIgnoringModifiers,
        modifiers: modifiers
      ),
      let text = event.characters
    {
      if !latestComposing.isEmpty, shiftEnglishComposition.isEmpty {
        _ = apply(bridge.enter(), client: client)
      }
      clearEnglishCompletion()
      shiftEnglishComposition.append(text)
      latestComposing = shiftEnglishComposition
      latestCandidates = [
        shiftEnglishComposition,
        String(text),
        shiftEnglishComposition.lowercased(),
      ]
      setMarkedComposition(shiftEnglishComposition, client: client)
      updateCandidateWindow(client: client)
      inputiaDebugLog("shiftEnglishComposition text=\(shiftEnglishComposition)")
      return true
    }
    if let navigation = InputiaShortcutClassifier.candidateNavigation(
      keyCode: event.keyCode,
      modifiers: modifiers,
      hasComposing: !latestComposing.isEmpty
    ) {
      return handleCandidateNavigation(navigation, client: client)
    }
    if modifiers.contains(.command) || modifiers.contains(.control) || modifiers.contains(.option) {
      cancelShiftInputModeGesture(reason: "blockingModifierKeyDown")
      return false
    }
    if modifiers.contains(.shift) {
      cancelShiftInputModeGesture(reason: "shiftModifiedKeyDown")
    }

    switch event.keyCode {
    case keyCodeDelete:
      if !shiftEnglishComposition.isEmpty {
        shiftEnglishComposition.removeLast()
        latestComposing = shiftEnglishComposition
        latestCandidates = shiftEnglishComposition.isEmpty
          ? []
          : [shiftEnglishComposition, String(shiftEnglishComposition.last!), shiftEnglishComposition.lowercased()]
        if shiftEnglishComposition.isEmpty {
          clearMarkedText(client)
          InputiaHost.candidatePanel?.hide()
        } else {
          setMarkedComposition(shiftEnglishComposition, client: client)
          updateCandidateWindow(client: client)
        }
        return true
      }
      let outcome = bridge.backspace()
      let handled = apply(outcome, client: client)
      updateEnglishCompletionAfterBackspace(outcome: outcome, client: client)
      return handled
    case keyCodeEscape:
      if !shiftEnglishComposition.isEmpty {
        clearShiftEnglishComposition(client: client)
        return true
      }
      #if INPUTIA_PAIRED_BUILD
      if personalEscapeConsumed { personalEscapeConsumed = false; return true }
      #endif
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
      if !shiftEnglishComposition.isEmpty {
        commitShiftEnglishComposition(client: client)
        return true
      }
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
      if !shiftEnglishComposition.isEmpty {
        commitShiftEnglishComposition(client: client)
        return true
      }
      #if INPUTIA_PAIRED_BUILD
      if hotwordOverlay != nil { return selectHotwordOverlay(displayed: 0, client: client) }
      if latestComposing.isEmpty && !personalPredictions.isEmpty {
        personalization.reset(); return false
      }
      if !personalCandidates.isEmpty, !candidatePanelExpanded, let first = personalCandidates.first {
        return choosePersonal(first, explicit: false, client: client)
      }
      #endif
      if candidatePanelExpanded, expandedActiveRowIndex > 0 {
        return commitExpandedCandidate(columnIndex: 0, client: client)
      }
      #if INPUTIA_PAIRED_BUILD
      if sharedChineseOrder != nil, !candidatePanelExpanded {
        return enqueueSharedChineseSelection(displayed: 0, client: client)
      }
      #endif
      if !latestComposing.isEmpty && !latestCandidates.isEmpty {
        return chooseNativeWithLearning(index: 0, explicit: false, client: client)
      }
      let outcome = bridge.space()
      let handled = apply(outcome, client: client)
      if outcome.mode == "English" && !outcome.consumed {
        learnAndClearEnglishCompletion(client: client)
      }
      return handled
    case keyCodeTab:
      if !shiftEnglishComposition.isEmpty {
        commitShiftEnglishComposition(client: client)
        return true
      }
      #if INPUTIA_PAIRED_BUILD
      if hotwordOverlay != nil { return selectHotwordOverlay(displayed: 0, client: client) }
      if latestComposing.isEmpty, let first = personalPredictions.first {
        return acceptPersonalPrediction(first, client: client)
      }
      #endif
      return commitFirstEnglishCompletion(client: client)
    default:
      break
    }

    if !shiftEnglishComposition.isEmpty {
      commitShiftEnglishComposition(client: client)
    }

    guard let text = event.characters, !text.isEmpty else {
      return false
    }

    #if INPUTIA_PAIRED_BUILD
    if hotwordOverlay != nil, !candidatePanelExpanded, !modifiers.contains(.shift),
      text.count == 1, let digit = Int(text), (1...9).contains(digit) {
      return selectHotwordOverlay(displayed: digit - 1, client: client)
    }
    if !personalCandidates.isEmpty, !candidatePanelExpanded,
      !modifiers.contains(.shift), text.count == 1, let digit = Int(text),
      digit > 0, digit <= 9 {
      guard digit <= min(latestCandidates.count, personalCandidates.count) else { return true }
      return choosePersonal(personalCandidates[digit - 1], explicit: true, client: client)
    }
    if personalCandidates.isEmpty, !latestComposing.isEmpty, !candidatePanelExpanded,
      !modifiers.contains(.shift), sharedChineseOrder == nil,
      text.count == 1, let digit = Int(text), digit > 0, digit <= latestCandidates.count {
      return chooseNativeWithLearning(index: digit - 1, explicit: true, client: client)
    }
    #endif
    if isDisplayedRawCompositionSelection(event, modifiers: modifiers) {
      return apply(bridge.enter(), client: client)
    }

    if let columnIndex = expandedCandidateDigitColumn(event, modifiers: modifiers) {
      return commitExpandedCandidate(columnIndex: columnIndex, client: client)
    }

    #if INPUTIA_PAIRED_BUILD
    if sharedChineseOrder != nil, !candidatePanelExpanded, !modifiers.contains(.shift),
      text.count == 1, let digit = Int(text), (1...9).contains(digit),
      sharedChineseOrder?.order.originalIndex(displayed: digit - 1) != nil {
      return enqueueSharedChineseSelection(displayed: digit - 1, client: client)
    }
    #endif

    var handled = false
    for character in text {
      let outcome = bridge.handle(character: character)
      handled = apply(outcome, client: client) || handled
      updateEnglishCompletionAfterCharacter(character, outcome: outcome, client: client)
    }
    return handled
  }

  private func apply(_ outcome: InputiaBridgeOutcome, client: IMKTextInput?) -> Bool {
    #if INPUTIA_PAIRED_BUILD
    if outcome.mode != "English" || !outcome.composing.isEmpty { retireWordSpan(reason: "mode_or_composition") }
    #endif
    clearClipboardRecall()
    let previousComposing = latestComposing
    inputiaDebugLog(
      "apply ok=\(outcome.ok) consumed=\(outcome.consumed) mode=\(outcome.mode) composing_count=\(outcome.composing.count) has_commit=\(outcome.commit != nil) candidate_count=\(outcome.candidates.count)"
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
      #if INPUTIA_PAIRED_BUILD
      if !previousComposing.isEmpty { MainActor.assumeIsolated { InputiaTypedCapture.shared.resetSegment() } }
      #endif
      candidatePanelExpanded = false
      InputiaHost.candidatePanel?.hide()
    } else {
      if outcome.composing != previousComposing {
        setMarkedComposition(outcome.composing, client: client)
      }
      updateCandidateWindow(client: client)
    }

    #if INPUTIA_PAIRED_BUILD
    if outcome.mode == "Chinese", !outcome.composing.isEmpty {
      _ = refreshHotwordPrefix(client: client)
      schedulePersonalization(client: client)
      scheduleSharedEnglishRefresh(client: client)
      scheduleManagedRank(client: client)
      prepareMemoryCommit(candidates: latestCandidates, prefix: "", client: client)
    }
    #endif
    return outcome.consumed
  }

  private func syncHostState(with outcome: InputiaBridgeOutcome) {
    #if INPUTIA_PAIRED_BUILD
    personalRefreshGeneration &+= 1
    personalization.invalidateView()
    hotwordOverlay = nil
    sharedChineseOrder = nil
    sharedChineseSelection.cancel()
    #endif
    let compositionChanged = latestComposing != outcome.composing
    #if INPUTIA_PAIRED_BUILD
    if compositionChanged { personalOrderLockedCode = nil }
    #endif
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

  private func chooseNativeWithLearning(index: Int, explicit: Bool, client: IMKTextInput, memoryValidated: Bool = false) -> Bool {
    #if INPUTIA_PAIRED_BUILD
    if !memoryValidated, memoryDisplayedTargets["rank"] != nil,
      memoryDisplayedComposing["rank"] == bridge.latestOutcome.composing {
      let before = bridge.latestOutcome
      return admitManagedSelection("rank", client: client, stillValid: { [weak self] in
        self?.bridge.latestOutcome.candidateIDs == before.candidateIDs && self?.bridge.latestOutcome.composing == before.composing
      }) { [weak self] in _ = self?.chooseNativeWithLearning(index: index, explicit: explicit, client: client, memoryValidated: true) }
    }
    let before = bridge.latestOutcome
    if before.candidates.indices.contains(index), before.candidateIDs.indices.contains(index),
      !before.candidateIDs[index].isEmpty, let pool = bridge.personalCandidatePool(),
      let candidate = pool.candidates.first(where: { $0.id == before.candidateIDs[index] && $0.text == before.candidates[index] }) {
      return choosePersonal(candidate, explicit: explicit, client: client)
    }
    #endif
    return apply(bridge.chooseCandidate(atZeroBasedIndex: index), client: client)
  }

  private func insertCommittedText(
    _ text: String,
    client: IMKTextInput,
    replacementRange: NSRange = emptyReplacementRange
  ) {
    #if INPUTIA_PAIRED_BUILD
    let memoryConfirmation = takeMemoryConfirmation(text: text, replacement: replacementRange, client: client)
    if memoryConfirmation != nil { retireWordSpan(reason: "fixed_plan_commit") }
    let origin = typedOriginBeforeInsertion(client)
    let before = client.selectedRange()
    let marked = client.markedRange()
    let start = replacementRange.location != NSNotFound ? replacementRange.location
      : (marked.location != NSNotFound ? marked.location : before.location)
    #endif
    client.insertText(text, replacementRange: replacementRange)
    #if INPUTIA_PAIRED_BUILD
    memoryConfirmation?()
    recordTypedCommit(text, client: client, start: start, origin: origin)
    recordPersonalCommit(text, client: client, start: start, origin: origin)
    #endif
  }

  #if INPUTIA_PAIRED_BUILD
  /// 只观察 Inputia 自己已完成的插入，不读取宿主全文、不监听其他输入。
  private func clearPersonalDisplay() {
    let rankedExpansion = expandedCandidateEntries.contains { $0.candidateID != nil }
    let visible = !personalCandidates.isEmpty || !personalPredictions.isEmpty || rankedExpansion
    personalCandidates = []; personalPredictions = []; personalCode = ""
    if rankedExpansion {
      expandedCandidateEntries = []; expandedCandidates = []; expandedActiveRowIndex = 0
      candidatePanelExpanded = false
    }
    guard visible else { return }
    if !latestComposing.isEmpty {
      let base = bridge.latestOutcome.candidates
      if let overlay = hotwordOverlay {
        hotwordOverlay = (overlay.code, overlay.words, base, overlay.snapshot, overlay.context)
        latestCandidates = Array((overlay.words + base).prefix(9))
      } else { latestCandidates = base }
      if let client = client() { updateCandidateWindow(client: client) }
    } else { InputiaHost.candidatePanel?.hide() }
  }

  /// 屏障不把旧排序换成新排序后继续接受同一数字；CAPI保留撤销选择闩锁。
  func clearManagedMemoryDisplay() {
    retireWordSpan(reason: "cache_barrier")
    retireMemoryCommit()
    memoryViewGeneration &+= 1
    memoryFieldID = nil
    memorySelectionIntent = nil
    let displayed = Array(memoryDisplayedTargets.values)
    memoryDisplayedTargets.removeAll(); memoryDisplayedComposing.removeAll()
    for snapshot in displayed {
      InputiaVoiceInputLauncher.releaseTarget(snapshot.targetID)
      removeRetiredVoiceTarget(snapshot.targetID)
    }
    bridge.cancelManagedMemoryRequests()
    personalRefreshGeneration &+= 1
    if let pending = personalDeferredOrigin { _ = personalInputDeferral.finish(pending.token) }
    personalDeferredOrigin = nil; personalRecoveryToken = nil; personalBoundaryProof = nil
    pendingPersonalSelection = nil; personalUndo = nil; personalPhrase.reset(); personalization.reset()
    personalCandidates = []; personalPredictions = []; personalCode = ""; personalOrderLockedCode = nil
    sharedEnglishSelection.cancel(); sharedChineseSelection.cancel(); sharedChineseOrder = nil
    sharedEnglishCandidates = [:]; explicitEnglishCandidates = [:]; hotwordOverlay = nil
    sharedEnglishRefreshQueued = false
    latestComposing = bridge.latestOutcome.composing
    latestCandidates = []; expandedCandidates = []; expandedCandidateEntries = []; expandedActiveRowIndex = 0
    recallCandidates = []; englishCompletionCandidates = []; englishCompletionPrefix = ""
    candidatePanelExpanded = false; cachedAppContext = nil; pushedAppContext = nil
    if InputiaHost.activeInputController === self { InputiaHost.candidatePanel?.hide() }
  }

  private func observePersonalKey(_ event: NSEvent, client: IMKTextInput) {
    personalEscapeConsumed = event.keyCode == keyCodeEscape && !personalPredictions.isEmpty
    // 只有明确Cmd+Z、紧邻自己的插入、原范围正文仍完全相符，才核对撤销回执。
    let undo = event.keyCode == 6 && event.modifierFlags.contains(.command)
      && !event.modifierFlags.contains(.shift)
    let undoOrigin = typedCompositionOrigin
    if undo, let record = personalUndo, let undoOrigin, undoOrigin.inputiaTarget == record.target,
      undoOrigin.isCurrentForTypedOrigin(client: client, controllerID: voiceControllerID,
        activationGeneration: voiceActivationGeneration),
      ProcessInfo.processInfo.systemUptime - record.time < 2,
      record.client == ObjectIdentifier(client as AnyObject), client.selectedRange() == record.after,
      record.inserted.utf16.count <= 128,
      client.attributedSubstring(from: NSRange(location: record.start, length: record.inserted.utf16.count))?.string == record.inserted {
      personalUndo = nil
      DispatchQueue.main.async { [weak self] in
        guard let self, InputiaHost.activeInputController === self, let current = self.client(),
          ObjectIdentifier(current as AnyObject) == record.client,
          current.selectedRange() == NSRange(location: record.start, length: 0),
          undoOrigin.isCurrentForTypedOrigin(client: current, controllerID: self.voiceControllerID,
            activationGeneration: self.voiceActivationGeneration),
          self.latestComposing.isEmpty,
          current.attributedSubstring(from: NSRange(location: record.start, length: record.inserted.utf16.count))?.string != record.inserted else { return }
        if record.nativeLearning { _ = self.bridge.undoRecentNativeLearning() }
        for receipt in record.receipts { self.personalization.undo(receipt) }
      }
    } else { personalUndo = nil }
    if latestComposing.isEmpty, let expected = personalExpectedSelection, client.selectedRange() != expected {
      personalization.reset(); personalPhrase.reset(); personalExpectedSelection = nil
    }
    let boundary = [UInt16(51), 53, 115, 116, 117, 119, 121, 123, 124, 125, 126].contains(event.keyCode)
      || !event.modifierFlags.intersection([.command, .control, .option]).isEmpty
    if isPersonalCandidateNavigation(event) {
      personalOrderLockedCode = latestComposing
      personalization.freezeResults()
    } else if boundary {
      personalPhrase.reset()
      if isPersonalCompositionBackspace(event) { personalization.invalidateView() }
      else if !undo { personalization.reset(); personalExpectedSelection = nil }
    }
    if !personalPredictions.isEmpty && event.keyCode != keyCodeTab {
      personalization.invalidateView()
      if event.keyCode == keyCodeSpace || event.keyCode == keyCodeReturn || event.keyCode == keyCodeKeypadEnter {
        personalization.reset()
      }
    }
  }

  private func schedulePersonalization(client: IMKTextInput) {
    let scheduleReason: String
    if !personalization.allowed { scheduleReason = "disabled" }
    else if InputiaHost.activeInputController !== self { scheduleReason = "not_active" }
    else if bridge.latestOutcome.mode != "Chinese" { scheduleReason = "mode" }
    else if bridge.latestOutcome.page != 0 { scheduleReason = "page" }
    else if candidatePanelExpanded { scheduleReason = "expanded" }
    else if !recallCandidates.isEmpty { scheduleReason = "recall" }
    else if personalPredictionPending { scheduleReason = "prediction_pending" }
    else if personalOrderLockedCode == latestComposing { scheduleReason = "selection_frozen" }
    else { scheduleReason = "ok" }
    InputiaPersonalizationDiagnostics.record("schedule", scheduleReason)
    guard personalization.allowed, InputiaHost.activeInputController === self,
      bridge.latestOutcome.mode == "Chinese", bridge.latestOutcome.page == 0, !candidatePanelExpanded,
      recallCandidates.isEmpty, !personalPredictionPending, personalOrderLockedCode != latestComposing else { return }
    let version = personalRefreshGeneration
    let code = latestComposing
    let page = bridge.latestOutcome.page
    let candidateIDs = bridge.latestOutcome.candidateIDs
    let candidateTexts = bridge.latestOutcome.candidates
    let schemaID = bridge.schemaID
    guard version == self.personalRefreshGeneration, self.latestComposing == code,
      InputiaPersonalContext.matchesFirstPage(page: self.bridge.latestOutcome.page, expectedPage: page,
        ids: self.bridge.latestOutcome.candidateIDs, expectedIDs: candidateIDs),
      InputiaHost.activeInputController === self, let current = self.client(),
      ObjectIdentifier(current as AnyObject) == ObjectIdentifier(client as AnyObject),
      let target = self.typedOriginBeforeInsertion(client), self.personalization.allowed,
      !self.candidatePanelExpanded else { return }
    self.personalization.bind(target)
    if code.isEmpty && self.personalization.context.text.isEmpty { return }
    let pool = code.isEmpty ? [] : (self.bridge.personalCandidatePool(limit: 32)?.candidates ?? [])
    let selection = client.selectedRange()
    self.personalization.query(target: target, code: code, schemaID: schemaID, candidates: pool) { [weak self] view in
      guard let self, version == self.personalRefreshGeneration, self.latestComposing == code,
        self.bridge.schemaID == schemaID, self.personalOrderLockedCode != code,
        self.bridge.latestOutcome.candidates == candidateTexts,
        InputiaPersonalContext.matchesFirstPage(page: self.bridge.latestOutcome.page, expectedPage: page,
          ids: self.bridge.latestOutcome.candidateIDs, expectedIDs: candidateIDs),
        self.bridge.latestOutcome.mode == "Chinese", !self.candidatePanelExpanded,
        InputiaHost.activeInputController === self, let live = self.client(),
        ObjectIdentifier(live as AnyObject) == ObjectIdentifier(client as AnyObject),
        live.selectedRange() == selection, self.typedCompositionOrigin?.inputiaTarget == target,
        self.typedCompositionOrigin?.isCurrentForTypedOrigin(client: live,
          controllerID: self.voiceControllerID, activationGeneration: self.voiceActivationGeneration) == true else { return }
      self.clearHotwordOverlay()
      self.sharedChineseOrder = nil
      if code.isEmpty {
        self.personalPredictions = view.predictions
        self.personalCandidates = []
        self.personalExpectedSelection = selection
        if view.predictions.isEmpty { InputiaHost.candidatePanel?.hide() }
        else {
          var rect = NSRect.zero
          live.attributes(forCharacterIndex: selection.location, lineHeightRectangle: &rect)
          InputiaHost.candidatePanel?.show(candidates: view.predictions.map(\.text), near: rect)
        }
      } else {
        var seen = Set<String>()
        self.personalCandidates = view.candidates.filter { seen.insert($0.text).inserted }
        self.personalPredictions = []; self.personalCode = code
        self.latestCandidates = Array(self.personalCandidates.prefix(max(1, self.bridge.latestOutcome.candidates.count))).map(\.text)
        self.refreshHotwordPrefix(client: live)
        self.updateCandidateWindow(client: live)
      }
    }
  }

  private func choosePersonal(_ candidate: InputiaPersonalCandidate, explicit: Bool, client: IMKTextInput) -> Bool {
    let code = latestComposing
    guard !code.isEmpty, bridge.latestOutcome.composing == code else { return false }
    if candidate.id.hasPrefix("learned:") {
      return acceptAdmittedPersonalCandidate(candidate, code: code, explicit: explicit, client: client)
    }
    let origin = typedOriginBeforeInsertion(client)
    personalRefreshGeneration &+= 1
    if let origin, personalization.allowed,
      let consumedCode = InputiaPersonalContext.consumedCode(code, length: candidate.consumed_len) {
      pendingPersonalSelection = PersonalSelection(target: origin,
        code: consumedCode, composing: code, schemaID: bridge.schemaID, candidate: candidate, explicit: explicit)
    }
    defer { pendingPersonalSelection = nil }
    let outcome = bridge.choosePersonalCandidate(id: candidate.id, text: candidate.text, composing: code)
    guard outcome.ok else { personalization.invalidateView(); return true }
    return apply(outcome, client: client)
  }

  private func recordPersonalCommit(_ text: String, client: IMKTextInput, start: Int, origin: InputiaVoiceTarget?) {
    let commitReason: String
    if pendingPersonalSelection == nil { commitReason = "missing_selection" }
    else if origin == nil { commitReason = "missing_origin" }
    else if origin != pendingPersonalSelection?.target { commitReason = "origin_mismatch" }
    else if pendingPersonalSelection?.candidate.text != text { commitReason = "text_mismatch" }
    else if start == NSNotFound { commitReason = "invalid_start" }
    else if client.selectedRange() != NSRange(location: start + text.utf16.count, length: 0) { commitReason = "selection_not_committed" }
    else { commitReason = "ok" }
    InputiaPersonalizationDiagnostics.record("commit", commitReason, flags: personalization.allowed ? 1 : 0)
    guard commitReason == "ok", let selection = pendingPersonalSelection, let origin else {
      personalization.reset(); personalPhrase.reset(); personalUndo = nil; return
    }
    guard personalization.allowed else { personalPhrase.reset(); personalUndo = nil; return }
    personalExpectedSelection = client.selectedRange()
    let phrase = personalPhrase.confirmed(target: origin.target_id, schema: selection.schemaID, epoch: personalization.epoch,
      composing: selection.composing, consumed: selection.code, remaining: latestComposing, text: text,
      previous: personalization.context.text, start: start, explicit: selection.explicit, rank: selection.candidate.base_rank)
    let receipt = personalization.accepted(target: origin, code: selection.code, schemaID: selection.schemaID, text: text,
      explicit: selection.explicit, rank: selection.candidate.base_rank) { [weak self] in
      guard let self, let live = self.client() else { return }
      self.schedulePersonalization(client: live)
    }
    if let receipt {
      var receipts = [receipt]
      if let phrase, let derived = personalization.accepted(target: origin, code: phrase.code,
        schemaID: selection.schemaID, text: phrase.text, explicit: phrase.explicit, rank: phrase.originalRank,
        previous: phrase.previous, appendContext: false, completion: {}) { receipts.append(derived) }
      personalUndo = PersonalUndo(receipts: receipts,
        nativeLearning: InputiaPersonalContext.hasNativeLearning(candidateID: selection.candidate.id),
        target: origin, inserted: text, start: start,
        after: client.selectedRange(), time: ProcessInfo.processInfo.systemUptime,
        client: ObjectIdentifier(client as AnyObject))
    } else { personalPhrase.reset(); personalUndo = nil }
  }

  private func acceptPersonalPrediction(_ prediction: InputiaPersonalPrediction, client: IMKTextInput) -> Bool {
    guard personalPredictions.contains(prediction) else { return false }
    return acceptAdmittedPersonalCandidate(InputiaPersonalCandidate(id: prediction.id, text: prediction.text,
      base_rank: 0, consumed_len: 0), code: "", explicit: true, client: client)
  }

  private func acceptAdmittedPersonalCandidate(_ candidate: InputiaPersonalCandidate, code: String,
    explicit: Bool, client: IMKTextInput) -> Bool {
    if personalPredictionPending { return true }
    guard latestComposing == code, bridge.latestOutcome.composing == code, personalization.allowed,
      let view = personalization.view, view.code == code, view.schemaID == bridge.schemaID,
      let origin = typedCompositionOrigin, origin.inputiaTarget == view.target,
      origin.isCurrentForTypedOrigin(client: client, controllerID: voiceControllerID,
        activationGeneration: voiceActivationGeneration),
      let selection = InputiaVoiceTargetSnapshot.validRange(client.selectedRange()), selection.length == 0,
      let token = personalInputDeferral.begin(now: ProcessInfo.processInfo.systemUptime) else { return false }
    let prediction = InputiaPersonalPrediction(id: candidate.id, text: candidate.text)
    let pending = PersonalDeferredOrigin(token: token, origin: origin, client: client,
      activation: voiceActivationGeneration, generation: localSelectionGeneration, epoch: personalization.epoch,
      schemaID: bridge.schemaID, code: code, selection: selection, marked: client.markedRange(), managedField: nil)
    personalDeferredOrigin = pending
    let candidateIDs = bridge.latestOutcome.candidateIDs
    DispatchQueue.main.asyncAfter(deadline: .now() + InputiaPersonalInputDeferral<NSEvent>.timeout) { [weak self] in
      guard let self, self.personalInputDeferral.expired(token, now: ProcessInfo.processInfo.systemUptime) else { return }
      self.finishDeferredPersonalInput(token)
    }
    // 服务核对学习证据和原字段；后续普通键暂存，不抢先修改正在准入的composition。
    personalization.admit(prediction, from: view) { [weak self] admitted in
      guard let self, self.personalInputDeferral.token == token else { return }
      guard let admitted, self.personalization.admissionIsCurrent(admitted),
        self.deferredPersonalScopeMatches(pending, composition: true),
        self.bridge.latestOutcome.candidateIDs == candidateIDs else {
        self.recoverDeferredPersonalInput(token); return
      }
      InputiaVoiceInputLauncher.targetBridge(.init(kind: "validate", target: origin.inputiaTarget, purpose: "personalization"),
        personalAdmissionDelivery: true) { [weak self] reply in
        guard let self, self.personalInputDeferral.token == token else { return }
        // targetBridge只返回有效ready租约；nil可能是拒绝或未知，二者均不能重放。
        guard let reply, !self.personalInputDeferral.expired(token, now: ProcessInfo.processInfo.systemUptime),
          self.personalization.admissionIsCurrent(admitted),
          self.personalReplayProofIsCurrent(reply, pending: pending, composition: true),
          self.bridge.latestOutcome.candidateIDs == candidateIDs else {
          self.finishDeferredPersonalInput(token); return
        }
        if !code.isEmpty {
          let cleared = self.bridge.escape()
          guard cleared.ok, cleared.composing.isEmpty, cleared.commit == nil else {
            self.finishDeferredPersonalInput(token, proof: reply); return
          }
          self.syncHostState(with: cleared)
        } else { self.personalization.invalidateView() }
        guard let queued = self.personalInputDeferral.finish(token) else { return }
        self.personalDeferredOrigin = nil; self.personalRecoveryToken = nil
        self.pendingPersonalSelection = PersonalSelection(target: origin.inputiaTarget, code: code,
          composing: code, schemaID: pending.schemaID, candidate: candidate, explicit: explicit)
        self.insertCommittedText(candidate.text, client: client,
          replacementRange: InputiaHostTextPolicy.commitReplacementRange(previousComposing: code, markedRange: pending.marked))
        self.pendingPersonalSelection = nil
        InputiaHost.candidatePanel?.hide()
        if self.personalBoundaryWait.isWaiting { self.personalBoundaryProof = reply }
        self.replayDeferredPersonalInput(queued, pending: pending, proof: reply)
      }
    }
    return true
  }

  private func deferredPersonalScopeMatches(_ pending: PersonalDeferredOrigin, composition: Bool) -> Bool {
    guard InputiaHost.activeInputController === self, voiceActivationGeneration == pending.activation,
      (pending.managedField != nil || personalization.epoch == pending.epoch), bridge.schemaID == pending.schemaID,
      (pending.managedField.map { memoryFieldID == $0 } ?? (typedCompositionOrigin === pending.origin)), let current = client(),
      ObjectIdentifier(current as AnyObject) == ObjectIdentifier(pending.client as AnyObject),
      pending.origin.isCurrentForTypedOrigin(client: current, controllerID: voiceControllerID,
        activationGeneration: pending.activation) else { return false }
    return !composition || (localSelectionGeneration == pending.generation && latestComposing == pending.code
      && bridge.latestOutcome.composing == pending.code && current.selectedRange() == pending.selection
      && current.markedRange() == pending.marked)
  }

  /// 可确认由IMK消费的composition编辑排队；宿主控制键留在当前物理事件栈上。
  private func deferPersonalInput(_ event: NSEvent, client: IMKTextInput) -> Bool? {
    guard let pending = personalDeferredOrigin else { return nil }
    let shouldQueue = InputiaPersonalDeferredEventPolicy.shouldQueue(event,
      candidateNavigation: isPersonalCandidateNavigation(event))
      || InputiaPersonalDeferredEventPolicy.canQueueEditing(event, after: personalInputDeferral.events,
        chineseMode: bridge.latestOutcome.mode == "Chinese")
    let sameClient = ObjectIdentifier(client as AnyObject) == ObjectIdentifier(pending.client as AnyObject)
    switch personalInputDeferral.offer(event, now: ProcessInfo.processInfo.systemUptime,
      scopeMatches: sameClient && deferredPersonalScopeMatches(pending, composition: true),
      boundary: !shouldQueue) {
    case .queued:
      // Shift/keyup仅延后本地手势观察；物理修饰键状态继续交给宿主，不注入系统事件。
      return event.type == .keyDown
    case .flush:
      // IMKTextInput没有公开的宿主命令派发API。保持当前Return/Tab/快捷键为原事件，
      // 只泵个人RPC专用mode；后来的物理键仍留在系统队列，不会越过本事件。
      guard !personalBoundaryWait.isWaiting else {
        finishDeferredPersonalInput(pending.token); return false
      }
      personalBoundaryProof = nil
      let result = personalBoundaryWait.wait(deadline: personalInputDeferral.deadline,
        pending: { self.personalPredictionPending },
        valid: { self.deferredPersonalScopeMatches(pending, composition: false) })
      if let remaining = personalDeferredOrigin { finishDeferredPersonalInput(remaining.token) }
      let mayProcess = result == .completed
        && personalReplayProofIsCurrent(personalBoundaryProof, pending: pending, composition: false)
      personalBoundaryProof = nil
      return mayProcess ? nil : false
    case .discard:
      finishDeferredPersonalInput(pending.token)
      return nil
    }
  }

  /// 准入失败后的恢复也必须重新核验原字段；沿用原750ms预算，失败或超时不重放。
  private func recoverDeferredPersonalInput(_ token: UInt64) {
    guard let pending = personalDeferredOrigin, pending.token == token,
      personalInputDeferral.token == token, personalRecoveryToken != token else { return }
    guard !personalInputDeferral.expired(token, now: ProcessInfo.processInfo.systemUptime),
      deferredPersonalScopeMatches(pending, composition: true) else {
      finishDeferredPersonalInput(token); return
    }
    personalRecoveryToken = token
    personalization.invalidateView()
    InputiaVoiceInputLauncher.targetBridge(.init(kind: "validate", target: pending.origin.inputiaTarget,
      purpose: "personalization"), personalAdmissionDelivery: true) { [weak self] reply in
      guard let self, self.personalRecoveryToken == token else { return }
      self.finishDeferredPersonalInput(token, proof: reply)
    }
  }

  private func finishDeferredPersonalInput(_ token: UInt64, proof: InputiaTargetBridgeReply? = nil) {
    guard let pending = personalDeferredOrigin, pending.token == token else { return }
    let withinBudget = !personalInputDeferral.expired(token, now: ProcessInfo.processInfo.systemUptime)
    guard let queued = personalInputDeferral.finish(token) else { return }
    personalDeferredOrigin = nil; personalRecoveryToken = nil
    let canReplay = withinBudget && personalReplayProofIsCurrent(proof, pending: pending, composition: true)
    personalization.invalidateView()
    if canReplay, let proof {
      if personalBoundaryWait.isWaiting { personalBoundaryProof = proof }
      replayDeferredPersonalInput(queued, pending: pending, proof: proof)
    }
    else { resetShiftInputModeSession(reason: "personal-deferral-cancelled") }
  }

  private func personalReplayProofIsCurrent(_ proof: InputiaTargetBridgeReply?,
    pending: PersonalDeferredOrigin, composition: Bool) -> Bool {
    let managedFieldMatches = pending.managedField.map { field in
      proof.map { reply in reply.field_instance.map { reply.server_instance + ":" + $0 == field } == true } ?? false
    } ?? true
    return InputiaPersonalInputDeferral<NSEvent>.allowsReplay(
      proofReady: proof?.ready == true && proof?.target == pending.origin.inputiaTarget && managedFieldMatches,
      proofDeadline: proof?.deadline, now: ProcessInfo.processInfo.systemUptime,
      leaseMatches: proof.map { InputiaPermissionLifecycle.shared.matchesService(server: $0.server_instance,
        epoch: $0.permission_epoch) && InputiaPermissionLifecycle.shared.permits(pending.origin.permissionEpoch) } == true,
      scopeMatches: deferredPersonalScopeMatches(pending, composition: composition))
  }

  private func replayDeferredPersonalInput(_ queued: [NSEvent], pending: PersonalDeferredOrigin,
    proof: InputiaTargetBridgeReply) {
    var selection = pending.client.selectedRange()
    var marked = pending.client.markedRange()
    for event in queued {
      guard personalReplayProofIsCurrent(proof, pending: pending, composition: false),
        pending.client.selectedRange() == selection, pending.client.markedRange() == marked else {
        resetShiftInputModeSession(reason: "personal-replay-invalidated"); return
      }
      // 已验证字段中的Escape只取消composition/context；仍需按序处理同字段后续键。
      personalReplayKeepsTypedOrigin = event.type == .keyDown && event.keyCode == keyCodeEscape
        && !latestComposing.isEmpty
      let handled = handle(event, client: pending.client)
      personalReplayKeepsTypedOrigin = false
      if !handled, InputiaPersonalDeferredEventPolicy.isPrintable(event), let text = event.characters,
        personalReplayProofIsCurrent(proof, pending: pending, composition: false) {
        // 已消费的普通文本不能再交回系统；只凭仍有效的原字段证明补齐未处理的空格/文字。
        pending.client.insertText(text, replacementRange: emptyReplacementRange)
      }
      selection = pending.client.selectedRange(); marked = pending.client.markedRange()
    }
  }

  private func isPersonalCandidateNavigation(_ event: NSEvent) -> Bool {
    !latestComposing.isEmpty && ([keyCodePageDown, keyCodePageUp].contains(event.keyCode)
      || InputiaShortcutClassifier.candidateNavigation(keyCode: event.keyCode,
        modifiers: event.modifierFlags.intersection(.deviceIndependentFlagsMask), hasComposing: true) != nil)
  }

  private func isPersonalCompositionBackspace(_ event: NSEvent) -> Bool {
    !latestComposing.isEmpty && event.keyCode == keyCodeDelete
      && event.modifierFlags.intersection([.command, .control, .option, .shift]).isEmpty
  }

  private func isPersonalRejectionShortcut(_ event: NSEvent) -> Bool {
    event.keyCode == keyCodeDelete
      && event.modifierFlags.intersection([.command, .control, .option, .shift]) == [.control]
  }

  @objc private func rejectPersonalCandidateFromMenu() {
    if let client = client() { _ = rejectPersonalCandidate(client: client) }
  }

  private func rejectPersonalCandidate(client: IMKTextInput) -> Bool {
    guard !latestComposing.isEmpty, !candidatePanelExpanded, !personalPredictionPending, hotwordOverlay == nil,
      let candidate = personalCandidates.first, let target = typedOriginBeforeInsertion(client),
      personalization.allowed else { return false }
    personalPhrase.reset(); personalUndo = nil; personalOrderLockedCode = nil
    _ = personalization.accepted(target: target, code: latestComposing, schemaID: bridge.schemaID,
      text: candidate.text, explicit: true, rank: candidate.base_rank, appendContext: false,
      operation: "reject") { [weak self] in
        guard let self, let current = self.client() else { return }
        self.schedulePersonalization(client: current)
      }
    return true
  }

  private func releaseTypedOriginIfUnowned(_ id: String) {
    guard typedRetainedOrigins[id] == nil, typedEventOrigin?.targetID != id,
      typedCompositionOrigin?.targetID != id, shortcutPreparedSnapshot?.targetID != id, voiceTargetSnapshots[id] == nil else { return }
    InputiaVoiceInputLauncher.releaseTarget(id)
  }

  private func discardTypedCompositionOrigin() {
    if let pending = personalDeferredOrigin { finishDeferredPersonalInput(pending.token) }
    personalization.reset(); personalPhrase.reset(); personalUndo = nil
    let old = typedCompositionOrigin
    typedCompositionOrigin = nil
    MainActor.assumeIsolated { InputiaTypedCapture.shared.resetSegment() }
    if let old { releaseTypedOriginIfUnowned(old.targetID) }
  }

  private func refreshTypedCompositionOrigin(client: IMKTextInput, boundary: Bool) {
    let old = typedCompositionOrigin
    let existingAllowed = old?.isCurrentForTypedOrigin(client: client, controllerID: voiceControllerID,
      activationGeneration: voiceActivationGeneration) == true
    let candidate = [shortcutPreparedSnapshot, typedEventOrigin].compactMap { $0 }.first {
      $0.inputiaTarget.field_id != nil && $0.isCurrentForShortcut(client: client, controllerID: voiceControllerID,
        activationGeneration: voiceActivationGeneration, isSensitiveApp: { _, _ in false }, windowTitle: { _ in .unavailable })
    }
    typedCompositionOrigin = InputiaTypedOriginLifetime.retain(old, existingAllowed: existingAllowed,
      candidate: candidate, boundary: boundary)
    if old?.targetID != typedCompositionOrigin?.targetID, let old {
      MainActor.assumeIsolated { InputiaTypedCapture.shared.resetSegment() }
      releaseTypedOriginIfUnowned(old.targetID)
    }
  }

  private func typedOriginBeforeInsertion(_ client: IMKTextInput) -> InputiaVoiceTarget? {
    refreshTypedCompositionOrigin(client: client, boundary: false)
    if let snapshot = typedCompositionOrigin,
      snapshot.isCurrentForTypedOrigin(client: client, controllerID: voiceControllerID,
        activationGeneration: voiceActivationGeneration) { return snapshot.inputiaTarget }
    for snapshot in [shortcutPreparedSnapshot, typedEventOrigin].compactMap({ $0 }) {
      if snapshot.inputiaTarget.field_id != nil,
        snapshot.isCurrentForShortcut(client: client, controllerID: voiceControllerID,
          activationGeneration: voiceActivationGeneration, isSensitiveApp: { _, _ in false },
          windowTitle: { _ in .unavailable }) {
        return snapshot.inputiaTarget
      }
    }
    // 后台预捕获只能供下一次提交使用；本次没有原字段证明便不收录。
    _ = shortcutRegistrationTarget()
    return nil
  }

  private func recordTypedCommit(_ text: String, client: IMKTextInput, start: Int, origin: InputiaVoiceTarget?) {
    guard Thread.isMainThread else { return }
    guard InputiaHost.activeInputController === self, let draft = origin, draft.field_id != nil,
      draft.controller_id == voiceControllerID, draft.activation_generation == voiceActivationGeneration,
      !IsSecureEventInputEnabled(), isCurrentInputiaSourceSelected(),
      let bundle = client.bundleIdentifier(), draft.source_app == bundle, start != NSNotFound,
      let end = InputiaVoiceTargetSnapshot.validRange(client.selectedRange()), end.length == 0,
      end.location >= start, end.location - start == text.utf16.count else {
      MainActor.assumeIsolated { InputiaTypedCapture.shared.resetSegment() }
      return
    }
    let identity = "\(ObjectIdentifier(client as AnyObject)):\(voiceControllerID):\(voiceActivationGeneration):\(bundle):\(draft.target_id)"
    let expiry = ProcessInfo.processInfo.systemUptime + 3
    typedRetainedOrigins[draft.target_id] = expiry
    DispatchQueue.main.asyncAfter(deadline: .now() + 3) { [weak self] in
      guard let self, self.typedRetainedOrigins[draft.target_id] == expiry else { return }
      self.typedRetainedOrigins.removeValue(forKey: draft.target_id)
      self.releaseTypedOriginIfUnowned(draft.target_id)
    }
    MainActor.assumeIsolated {
      InputiaTypedCapture.shared.committed(text: text, draft: draft, identity: identity,
        start: start, end: end.location)
    }
  }

  private func reportShortcutReadiness(_ reason: String) {
    guard reason != shortcutReadinessReason else { return }
    shortcutReadinessReason = reason
    shiftDiagnostic.notice("shortcut_target reason=\(reason, privacy: .public)")
  }

  func shortcutRegistrationTarget() -> InputiaVoiceTarget? {
    sharedTargetReady = false
    guard InputiaHost.activeInputController === self, isCurrentInputiaSourceSelected() else {
      reportShortcutReadiness("inactive_source"); return nil
    }
    synchronizePermissionEpoch()
    guard InputiaPermissionLifecycle.shared.isReady else { reportShortcutReadiness("accessibility_permission_required"); return nil }
    guard !IsSecureEventInputEnabled() else { reportShortcutReadiness("secure_input_enabled"); return nil }
    guard let client = client() else { reportShortcutReadiness("missing_imk_client"); return nil }
    if let snapshot = shortcutPreparedSnapshot, snapshot.activationGeneration == voiceActivationGeneration,
      sharedPreparedClientIdentity == ObjectIdentifier(client as AnyObject),
      snapshot.reusableForShortcut, snapshot.compositionGeneration == localCompositionGeneration,
      snapshot.localSelectionGeneration == localSelectionGeneration,
      snapshot.isCurrentForShortcut(client: client, controllerID: voiceControllerID,
        activationGeneration: voiceActivationGeneration, isSensitiveApp: { _, _ in false }, windowTitle: { _ in .unavailable }) {
      reportShortcutReadiness("ready")
      sharedTargetReady = true
      return snapshot.inputiaTarget
    }
    discardPreparedVoiceTarget()
    // 仅限制后台预捕获；显式录音与派发前仍独立重新验证当前字段。
    guard InputiaVoiceTargetSnapshot.shouldAttemptPreCapture(
      now: ProcessInfo.processInfo.systemUptime, retryDeadline: voicePreCaptureRetryDeadline
    ) else { return nil }
    guard !targetCapturePending else { return nil }
    targetCapturePending = true
    prepareUnifiedVoiceTarget(client: client) { [weak self] target in
      guard let self else { return }
      self.targetCapturePending = false
      guard let target, let snapshot = self.voiceTargetSnapshots.removeValue(forKey: target.target_id) else {
        if let target { InputiaVoiceInputLauncher.releaseTarget(target.target_id) }
        self.voicePreCaptureRetryDeadline = InputiaVoiceTargetSnapshot.preCaptureRetryDeadline(after: ProcessInfo.processInfo.systemUptime)
        return
      }
      self.shortcutPreparedSnapshot = snapshot
      self.sharedPreparedClientIdentity = ObjectIdentifier(client as AnyObject)
      self.sharedTargetReady = true
    }
    return nil
  }

  func acceptUnifiedShortcut(_ trigger: InputiaHostShortcutTrigger, completion: @escaping (Bool) -> Void) {
    synchronizePermissionEpoch()
    guard InputiaPermissionLifecycle.shared.isReady, trigger.starts_session, InputiaHost.activeInputController === self,
      isCurrentInputiaSourceSelected(), let snapshot = shortcutPreparedSnapshot,
      snapshot.inputiaTarget == trigger.target, snapshot.compositionGeneration == localCompositionGeneration,
      snapshot.localSelectionGeneration == localSelectionGeneration,
      snapshot.isCurrentForShortcut(client: client(), controllerID: voiceControllerID,
        activationGeneration: voiceActivationGeneration,
        isSensitiveApp: { self.bridge.isSensitiveApp(bundleId: $0, windowTitle: $1) },
        windowTitle: { self.checkedWindowTitle(forBundleId: $0) })
    else { completion(false); return }
    voiceTargetSnapshots[snapshot.targetID] = snapshot
    voicePermissionEpochs[snapshot.targetID] = permissionEpoch
    InputiaVoiceInputLauncher.sendUnifiedShortcutTrigger(trigger, deliver: { [weak self] delivery, ack in
      guard let self else { ack("pending_target"); return }
      self.deliverUnifiedVoice(delivery, acknowledge: ack)
    }, status: { [weak self] message in self?.voiceStatus = message }, completion: completion)
  }

  private func currentSharedTerms(client: IMKTextInput) -> InputiaSharedTermsSnapshot? {
    guard InputiaPermissionLifecycle.shared.permits(permissionEpoch), sharedTargetReady, InputiaHost.activeInputController === self,
      sharedPreparedClientIdentity == ObjectIdentifier(client as AnyObject),
      let snapshot = shortcutPreparedSnapshot, snapshot.reusableForShortcut,
      snapshot.controllerID == voiceControllerID,
      snapshot.activationGeneration == voiceActivationGeneration else { return nil }
    return InputiaSharedTermsMemory.shared.current(target: snapshot.inputiaTarget)
  }

  /// 只在异步主线程任务调用；键盘回调不得等待实时 AX/窗口隐私检查。
  private func liveSharedTerms(client: IMKTextInput) -> InputiaSharedTermsSnapshot? {
    guard InputiaPermissionLifecycle.shared.permits(permissionEpoch), !IsSecureEventInputEnabled(), let snapshot = shortcutPreparedSnapshot,
      snapshot.isCurrentForShortcut(client: client, controllerID: voiceControllerID,
        activationGeneration: voiceActivationGeneration,
        isSensitiveApp: { self.bridge.isSensitiveApp(bundleId: $0, windowTitle: $1) },
        windowTitle: { self.checkedWindowTitle(forBundleId: $0) }) else { return nil }
    return currentSharedTerms(client: client)
  }

  private func scheduleSharedEnglishRefresh(client: IMKTextInput) {
    guard !sharedEnglishRefreshQueued else { return }
    sharedEnglishRefreshQueued = true
    DispatchQueue.main.async { [weak self] in
      guard let self else { return }
      self.sharedEnglishRefreshQueued = false
      guard InputiaHost.activeInputController === self,
        self.sharedPreparedClientIdentity == ObjectIdentifier(client as AnyObject),
        !self.candidatePanelExpanded else { return }
      if self.bridge.latestOutcome.mode == "English", self.latestComposing.isEmpty,
        self.englishCompletionPrefix.count >= 2 {
        self.refreshEnglishCompletions(client: client, includeShared: true)
      } else if self.bridge.latestOutcome.mode == "Chinese", !self.latestComposing.isEmpty {
        self.refreshSharedChineseCandidates(client: client)
      }
    }
  }

  private func enqueueSharedEnglishSelection(_ candidate: String, identity: String, client: IMKTextInput) -> Bool {
    guard !sharedEnglishSelection.hasPending else { return true }
    guard let snapshot = shortcutPreparedSnapshot,
      let shared = currentSharedTerms(client: client), shared.identity == identity,
      shared.englishCandidates(prefix: englishCompletionPrefix).contains(candidate) else {
      clearSharedEnglishCandidates()
      return true
    }
    let prefix = englishCompletionPrefix
    let selection = client.selectedRange()
    guard let intent = sharedEnglishSelection.begin(.init(prefix: prefix, targetID: snapshot.targetID,
      cacheIdentity: identity, clientIdentity: ObjectIdentifier(client as AnyObject), activation: voiceActivationGeneration)) else { return true }
    DispatchQueue.main.async { [weak self] in
      guard let self, self.sharedEnglishSelection.isPending(intent) else { return }
      guard let liveClient = self.client(), ObjectIdentifier(liveClient as AnyObject) == ObjectIdentifier(client as AnyObject),
        liveClient.selectedRange() == selection,
        self.englishCompletionPrefix == prefix, self.bridge.latestOutcome.mode == "English",
        self.latestComposing.isEmpty, self.shortcutPreparedSnapshot === snapshot,
        let current = self.liveSharedTerms(client: client), current.identity == identity,
        current.englishCandidates(prefix: prefix).contains(candidate), let suffix = self.completionSuffix(for: candidate), !suffix.isEmpty,
        self.sharedEnglishSelection.isPending(intent),
        self.bridge.latestOutcome.mode == "English", self.latestComposing.isEmpty,
        !IsSecureEventInputEnabled(),
        self.currentSharedTerms(client: client)?.identity == identity else {
        self.sharedEnglishSelection.cancel()
        InputiaSharedTermsMemory.shared.clear()
        return
      }
      // 先消耗唯一选择意图，再提交一次；不进入旧学习库，也不自动重放。
      guard self.sharedEnglishSelection.consume(intent, context: .init(prefix: self.englishCompletionPrefix,
        targetID: current.target.target_id, cacheIdentity: current.identity,
        clientIdentity: ObjectIdentifier(client as AnyObject), activation: self.voiceActivationGeneration),
        gateAllowed: true) else { InputiaSharedTermsMemory.shared.clear(); return }
      let origin = self.typedOriginBeforeInsertion(client)
      let typedStart = client.selectedRange().location
      client.insertText(suffix, replacementRange: emptyReplacementRange)
      self.recordTypedCommit(suffix, client: client, start: typedStart, origin: origin)
      self.clearEnglishCompletion()
    }
    return true
  }

  private func clearHotwordOverlay() {
    guard let overlay = hotwordOverlay else { return }
    hotwordOverlay = nil
    if latestComposing == overlay.code {
      latestCandidates = overlay.base
      if !candidatePanelExpanded {
        if latestCandidates.isEmpty { InputiaHost.candidatePanel?.hide() }
        else { InputiaHost.candidatePanel?.show(candidates: latestCandidates, near: chineseCandidateRect) }
      }
    }
  }

  private func explicitInputAllowed(client: IMKTextInput) -> Bool {
    InputiaHost.activeInputController === self && isCurrentInputiaSourceSelected()
      && self.client().map { ObjectIdentifier($0 as AnyObject) == ObjectIdentifier(client as AnyObject) } == true
      && !IsSecureEventInputEnabled() && !InputiaSecureDirectPolicy.shouldUseSecureDirectMode(context: appContext(for: client))
  }

  private func explicitSelectionContext(client: IMKTextInput, snapshot: InputiaExplicitHotwords.Snapshot) -> InputiaExplicitSelectionContext {
    .init(client: ObjectIdentifier(client as AnyObject), activation: voiceActivationGeneration,
      mode: bridge.latestOutcome.mode, code: bridge.latestOutcome.mode == "English" ? englishCompletionPrefix : bridge.latestOutcome.composing,
      naturalDoublePinyin: bridge.usesNaturalDoublePinyin, selection: client.selectedRange(), generation: snapshot.generation)
  }

  func refreshExplicitHotwords() {
    clearHotwordOverlay()
    guard let client = client(), explicitInputAllowed(client: client) else {
      explicitEnglishCandidates = [:]; return
    }
    if bridge.latestOutcome.mode == "Chinese" { _ = refreshHotwordPrefix(client: client) }
    else if !englishCompletionPrefix.isEmpty { refreshEnglishCompletions(client: client) }
  }

  @discardableResult private func refreshHotwordPrefix(client: IMKTextInput) -> Bool {
    clearHotwordOverlay()
    let current = bridge.latestOutcome
    guard current.mode == "Chinese", !current.composing.isEmpty, !candidatePanelExpanded,
      shiftEnglishComposition.isEmpty, explicitInputAllowed(client: client) else { return false }
    let snapshot = InputiaExplicitHotwords.shared.snapshot()
    let words = Array(InputiaHotwordPrefix.candidates(snapshot.words, code: current.composing, naturalDoublePinyin: bridge.usesNaturalDoublePinyin).prefix(3))
    guard !words.isEmpty else { return false }
    let base = latestCandidates
    hotwordOverlay = (current.composing, words, base, snapshot, explicitSelectionContext(client: client, snapshot: snapshot))
    latestCandidates = Array((words + base).prefix(9))
    updateCandidateWindow(client: client)
    return true
  }

  private func selectHotwordOverlay(displayed: Int, client: IMKTextInput) -> Bool {
    guard let overlay = hotwordOverlay, latestComposing == overlay.code,
      displayed >= 0, displayed < latestCandidates.count else { return true }
    if displayed >= overlay.words.count {
      let index = displayed - overlay.words.count
      clearHotwordOverlay()
      if personalCandidates.indices.contains(index) {
        return choosePersonal(personalCandidates[index], explicit: true, client: client)
      }
      if sharedChineseOrder != nil { return enqueueSharedChineseSelection(displayed: index, client: client) }
      return chooseNativeWithLearning(index: index, explicit: true, client: client)
    }
    let word = overlay.words[displayed]
    let expected = overlay.context
    let current = InputiaExplicitHotwords.shared.snapshot()
    guard expected.admits(current: explicitSelectionContext(client: client, snapshot: current), word: word,
      snapshot: current, secure: IsSecureEventInputEnabled(), active: explicitInputAllowed(client: client)),
      bridge.latestOutcome.mode == "Chinese", bridge.latestOutcome.composing == overlay.code,
      InputiaHotwordPrefix.candidates(current.words, code: overlay.code, naturalDoublePinyin: bridge.usesNaturalDoublePinyin).contains(word)
    else { clearHotwordOverlay(); return true }
    let replacement = InputiaHostTextPolicy.commitReplacementRange(previousComposing: overlay.code, markedRange: client.markedRange())
    let cancelled = bridge.escape()
    guard cancelled.ok, cancelled.composing.isEmpty, cancelled.commit == nil else { return true }
    syncHostState(with: cancelled)
    // 用户手动选择的基础词汇由当前IMK客户端提交，不进入语音/学习/外传链路。
    client.insertText(word, replacementRange: replacement)
    InputiaHost.candidatePanel?.hide()
    return true
  }

  private func sharedChineseCoreMatches(_ value: (order: InputiaSharedCandidateOrder, candidates: [String], identity: String, target: InputiaVoiceTarget)) -> Bool {
    let current = bridge.latestOutcome
    return value.order.matches(mode: current.mode, composing: current.composing, page: current.page,
      candidates: current.candidates, originalCandidates: value.candidates)
      && latestComposing == value.order.composing && !candidatePanelExpanded
  }

  private func clearSharedChineseCandidates() {
    sharedChineseSelection.cancel()
    if let overlay = hotwordOverlay {
      sharedChineseOrder = nil
      let base = personalCandidates.isEmpty ? bridge.latestOutcome.candidates : Array(personalCandidates.prefix(max(1, bridge.latestOutcome.candidates.count))).map(\.text)
      hotwordOverlay = (overlay.code, overlay.words, base, overlay.snapshot, overlay.context)
      latestCandidates = Array((overlay.words + base).prefix(9))
      if let client = client() { updateCandidateWindow(client: client) }
      return
    }
    if !personalCandidates.isEmpty { sharedChineseOrder = nil; return }
    guard sharedChineseOrder != nil else { return }
    sharedChineseOrder = nil
    let current = bridge.latestOutcome
    guard current.mode == "Chinese", !current.composing.isEmpty else { return }
    latestCandidates = current.candidates
    if !candidatePanelExpanded {
      InputiaHost.candidatePanel?.show(candidates: current.candidates, near: chineseCandidateRect)
    }
  }

  private func refreshSharedChineseCandidates(client: IMKTextInput) {
    clearHotwordOverlay()
    if refreshHotwordPrefix(client: client) { return }
    guard personalCandidates.isEmpty, !personalization.allowed else { return }
    clearSharedChineseCandidates()
    guard !candidatePanelExpanded, let shared = liveSharedTerms(client: client) else {
      InputiaSharedTermsMemory.shared.clear(); return
    }
    let before = bridge.latestOutcome
    guard before.mode == "Chinese", !before.composing.isEmpty,
      let order = bridge.sharedCandidateOrder(terms: shared.terms),
      currentSharedTerms(client: client)?.identity == shared.identity,
      before.candidates == bridge.latestOutcome.candidates,
      before.composing == bridge.latestOutcome.composing, before.page == bridge.latestOutcome.page,
      order.indices != Array(before.candidates.indices) else { return }
    sharedChineseOrder = (order, before.candidates, shared.identity, shared.target)
    latestCandidates = order.indices.map { before.candidates[$0] }
    // 不调用 apply/syncHostState，marked text 与 Rime 原页保持不变。
    updateCandidateWindow(client: client)
  }

  private func enqueueSharedChineseSelection(displayed: Int, client: IMKTextInput) -> Bool {
    guard !sharedChineseSelection.hasPending else { return true }
    guard let mapping = sharedChineseOrder, sharedChineseCoreMatches(mapping),
      let originalIndex = mapping.order.originalIndex(displayed: displayed),
      let snapshot = shortcutPreparedSnapshot,
      currentSharedTerms(client: client)?.identity == mapping.identity else {
      clearSharedChineseCandidates(); return true
    }
    guard let intent = sharedChineseSelection.begin(.init(prefix: mapping.order.composing,
      targetID: mapping.target.target_id, cacheIdentity: mapping.identity,
      clientIdentity: ObjectIdentifier(client as AnyObject), activation: voiceActivationGeneration)) else { return true }
    DispatchQueue.main.async { [weak self] in
      guard let self, self.sharedChineseSelection.isPending(intent) else { return }
      guard let liveClient = self.client(), ObjectIdentifier(liveClient as AnyObject) == ObjectIdentifier(client as AnyObject),
        self.shortcutPreparedSnapshot === snapshot, self.sharedChineseCoreMatches(mapping),
        self.sharedChineseOrder?.identity == mapping.identity,
        self.sharedChineseOrder?.order.indices == mapping.order.indices,
        let shared = self.liveSharedTerms(client: client), shared.identity == mapping.identity,
        shared.target == mapping.target, self.sharedChineseCoreMatches(mapping),
        !IsSecureEventInputEnabled(), self.currentSharedTerms(client: client)?.identity == mapping.identity,
        self.sharedChineseSelection.consume(intent, context: .init(prefix: self.bridge.latestOutcome.composing,
          targetID: shared.target.target_id, cacheIdentity: shared.identity,
          clientIdentity: ObjectIdentifier(client as AnyObject), activation: self.voiceActivationGeneration), gateAllowed: true)
      else { self.sharedChineseSelection.cancel(); InputiaSharedTermsMemory.shared.clear(); return }
      // 原索引交回 Rime，自身不插字符串；保留部分消费、剩余组合及既有用户选择学习。
      _ = self.chooseNativeWithLearning(index: originalIndex, explicit: true, client: client)
    }
    return true
  }

  func acceptSharedTerms(_ terms: InputiaSharedTermsSnapshot, ticket: UInt64) {
    guard InputiaSharedTermsMemory.shared.ticket() == ticket else { return }
    guard Thread.isMainThread, sharedTargetReady, InputiaHost.activeInputController === self,
      let currentClient = client(), sharedPreparedClientIdentity == ObjectIdentifier(currentClient as AnyObject),
      let snapshot = shortcutPreparedSnapshot, snapshot.reusableForShortcut,
      snapshot.controllerID == voiceControllerID,
      snapshot.activationGeneration == voiceActivationGeneration,
      snapshot.inputiaTarget == terms.target,
      !IsSecureEventInputEnabled(),
      snapshot.isCurrentForShortcut(client: currentClient, controllerID: voiceControllerID,
        activationGeneration: voiceActivationGeneration,
        isSensitiveApp: { self.bridge.isSensitiveApp(bundleId: $0, windowTitle: $1) },
        windowTitle: { self.checkedWindowTitle(forBundleId: $0) }),
      InputiaSharedTermsMemory.shared.install(terms, ticket: ticket) else {
      if InputiaSharedTermsMemory.shared.ticket() == ticket { InputiaSharedTermsMemory.shared.clear() }
      return
    }
    if (!englishCompletionPrefix.isEmpty || !latestComposing.isEmpty), let client = client() {
      scheduleSharedEnglishRefresh(client: client)
    }
    DispatchQueue.main.asyncAfter(deadline: .now() + max(0, terms.expiresAt - ProcessInfo.processInfo.systemUptime)) {
      InputiaSharedTermsMemory.shared.expire(identity: terms.identity)
    }
  }

  func clearSharedEnglishCandidates() {
    sharedEnglishSelection.cancel()
    clearSharedChineseCandidates()
    guard !sharedEnglishCandidates.isEmpty else { return }
    let removed = Set(sharedEnglishCandidates.keys).subtracting(explicitEnglishCandidates.keys)
    sharedEnglishCandidates = [:]
    englishCompletionCandidates.removeAll { removed.contains($0) }
    guard latestComposing.isEmpty, recallCandidates.isEmpty else { return }
    latestCandidates = englishCompletionCandidates
    if englishCompletionCandidates.isEmpty { InputiaHost.candidatePanel?.hide() }
    else { InputiaHost.candidatePanel?.show(candidates: englishCompletionCandidates, near: englishCompletionRect) }
  }

  private func retireMemoryCommit() {
    memoryCommitExpiry?.cancel(); memoryCommitExpiry = nil
    if let pending = memoryPreparedCommit {
      InputiaVoiceInputLauncher.releaseTarget(pending.target.target_id)
      removeRetiredVoiceTarget(pending.target.target_id)
    }
    memoryPreparedCommit = nil
  }

  /// 候选已可见后异步准备；用户按键从不等待 IPC，也不追认迟到的许可。
  private func prepareMemoryCommit(candidates: [String], prefix: String, client: IMKTextInput) {
    retireMemoryCommit()
    guard bridge.managedMemoryEnabled, !candidates.isEmpty, candidates.count <= 64 else { return }
    let range = prefix.isEmpty ? client.markedRange() : client.selectedRange()
    guard range.location != NSNotFound, range.location >= 0, range.length >= 0,
      prefix.isEmpty || range.length == 0 else { return }
    guard let replaced = prefix.isEmpty ? client.attributedSubstring(from: range)?.string : "" else { return }
    let inserted = candidates.compactMap { candidate -> String? in
      if prefix.isEmpty { return candidate }
      guard candidate.lowercased().hasPrefix(prefix.lowercased()), candidate.count > prefix.count else { return nil }
      return String(candidate.dropFirst(prefix.count))
    }
    guard inserted.count == candidates.count, inserted.reduce(0, { $0 + $1.utf8.count }) <= 65_536 else { return }
    let request = InputiaMemoryFixedRequest(replacement: .init(location: UInt64(range.location), length: UInt64(range.length)),
      replaced_text: replaced, retained_prefix: prefix,
      plans: inserted.enumerated().map { .init(candidate_id: "candidate-\($0.offset)", inserted_text: $0.element) })
    let generation = memoryViewGeneration, activation = voiceActivationGeneration, identity = ObjectIdentifier(client as AnyObject)
    prepareUnifiedVoiceTarget(client: client) { [weak self] target in
      guard let self, let target else { return }
      let started = ProcessInfo.processInfo.systemUptime
      guard target.field_id != nil, self.memoryViewGeneration == generation else {
        InputiaVoiceInputLauncher.releaseTarget(target.target_id); self.removeRetiredVoiceTarget(target.target_id); return
      }
      InputiaVoiceBridge.shared.management(.init(kind: "prepare_commit", target: InputiaMemoryTarget(target), request: request)) { [weak self] result in
        guard let self, let client = self.client(), self.memoryViewGeneration == generation,
          self.voiceActivationGeneration == activation, InputiaHost.activeInputController === self,
          ObjectIdentifier(client as AnyObject) == identity,
          (prefix.isEmpty ? client.markedRange() : client.selectedRange()) == range,
          case .success(let reply) = result, reply.code == nil, reply.result?.kind == "prepared_commit",
          let permit = reply.result?.permit,
          (try? permit.validate(request: request, started: started, now: ProcessInfo.processInfo.systemUptime)) != nil else {
          InputiaVoiceInputLauncher.releaseTarget(target.target_id); self?.removeRetiredVoiceTarget(target.target_id); return
        }
        self.retireMemoryCommit()
        self.memoryPreparedCommit = .init(target: target,
          policy: .init(server_instance: reply.server_instance, profile_id: reply.profile_id, policy_epoch: reply.policy_epoch),
          request: request, permit: permit, started: started, client: identity, activation: activation)
        let commitID = permit.commit_id
        let expiry = DispatchWorkItem { [weak self] in
          guard self?.memoryPreparedCommit?.permit.commit_id == commitID else { return }
          self?.retireMemoryCommit()
        }
        self.memoryCommitExpiry = expiry
        DispatchQueue.main.asyncAfter(deadline: .now() + max(0, started + Double(permit.max_age_ms) / 1000 - ProcessInfo.processInfo.systemUptime), execute: expiry)
      }
    }
  }

  private func takeMemoryConfirmation(text: String, replacement: NSRange, client: IMKTextInput) -> (() -> Void)? {
    guard let pending = memoryPreparedCommit else { return nil }
    memoryCommitExpiry?.cancel(); memoryCommitExpiry = nil
    memoryPreparedCommit = nil
    let release = { [weak self] in
      InputiaVoiceInputLauncher.releaseTarget(pending.target.target_id)
      self?.removeRetiredVoiceTarget(pending.target.target_id)
    }
    let actual = replacement.location == NSNotFound ? client.selectedRange() : replacement
    guard InputiaHost.activeInputController === self, pending.activation == voiceActivationGeneration,
      pending.client == ObjectIdentifier(client as AnyObject), !IsSecureEventInputEnabled(),
      InputiaPermissionLifecycle.shared.isReady, actual.location >= 0, actual.length >= 0,
      pending.request.replacement == InputiaMemoryRange(location: UInt64(actual.location), length: UInt64(actual.length)),
      pending.request.retained_prefix.isEmpty || pending.request.retained_prefix == englishCompletionPrefix,
      pending.request.replaced_text.isEmpty || client.attributedSubstring(from: actual)?.string == pending.request.replaced_text,
      (try? pending.permit.validate(request: pending.request, started: pending.started, now: ProcessInfo.processInfo.systemUptime)) != nil,
      let candidate = pending.request.plans.first(where: { $0.inserted_text == text }),
      let plan = pending.permit.plans.first(where: { $0.candidate_id == candidate.candidate_id }) else { release(); return nil }
    return {
      let operation = pending.permit.operation(plan: plan.plan_id)
      InputiaVoiceBridge.shared.management(.init(kind: "confirm_commit", target: InputiaMemoryTarget(pending.target),
        operation_id: operation, commit_id: pending.permit.commit_id, plan_id: plan.plan_id), expectedEpoch: pending.policy.policy_epoch) { result in
        defer { release() }
        guard case .success(let reply) = result, reply.code == nil, reply.result?.kind == "learn",
          let receipt = reply.result?.receipt, receipt.operation_id == operation,
          receipt.applied_at_epoch == pending.policy.policy_epoch else {
          InputiaPersonalizationDiagnostics.record("memory_commit", "unconfirmed"); return
        }
        InputiaPersonalizationDiagnostics.record("memory_commit", receipt.state)
      }
    }
  }

  /// 目标由认证主程序采集；回复只属于这次输入、这个控制器和真实字段。
  private func requestManagedMemory(_ query: InputiaMemoryQuery, client: IMKTextInput,
                                    completion: @escaping () -> Void) {
    guard bridge.managedMemoryEnabled, InputiaHost.activeInputController === self else { return }
    let generation = memoryViewGeneration, activation = voiceActivationGeneration
    let composing = bridge.latestOutcome.composing
    let selection = client.selectedRange(), identity = ObjectIdentifier(client as AnyObject)
    let current = { [weak self] in
      guard let self, let client = self.client() else { return false }
      return InputiaHost.activeInputController === self && self.memoryViewGeneration == generation
        && self.voiceActivationGeneration == activation && self.bridge.latestOutcome.composing == composing
        && ObjectIdentifier(client as AnyObject) == identity && client.selectedRange() == selection
        && !IsSecureEventInputEnabled() && InputiaPermissionLifecycle.shared.isReady
    }
    prepareUnifiedVoiceTarget(client: client) { [weak self] target in
      guard let self, let target else { return }
      guard current(), let field = self.memoryTargetFields[target.target_id],
        let snapshot = self.voiceTargetSnapshots[target.target_id] else {
        InputiaVoiceInputLauncher.releaseTarget(target.target_id); self.removeRetiredVoiceTarget(target.target_id); return
      }
      guard self.memoryFieldID == field else {
        InputiaVoiceInputLauncher.releaseTarget(target.target_id); self.removeRetiredVoiceTarget(target.target_id); return
      }
      self.bridge.requestManagedMemory(query, target: target, stillCurrent: current) { [weak self] installed in
        guard let self else { InputiaVoiceInputLauncher.releaseTarget(target.target_id); return }
        guard installed, current() else {
          InputiaVoiceInputLauncher.releaseTarget(target.target_id); self.removeRetiredVoiceTarget(target.target_id); return
        }
        if let old = self.memoryDisplayedTargets[query.kind], old.targetID != target.target_id {
          InputiaVoiceInputLauncher.releaseTarget(old.targetID); self.removeRetiredVoiceTarget(old.targetID)
        }
        self.memoryDisplayedTargets[query.kind] = snapshot
        self.memoryDisplayedComposing[query.kind] = composing
        completion()
      }
    }
  }

  private func managedDisplayCurrent(_ kind: String, client: IMKTextInput) -> Bool {
    guard let snapshot = memoryDisplayedTargets[kind], InputiaHost.activeInputController === self else { return false }
    return snapshot.isCurrentForShortcut(client: client, controllerID: voiceControllerID,
      activationGeneration: voiceActivationGeneration,
      isSensitiveApp: { self.bridge.isSensitiveApp(bundleId: $0, windowTitle: $1) },
      windowTitle: { self.checkedWindowTitle(forBundleId: $0) })
  }

  /// 选择旧显示映射前核验原target，绝不重新capture新字段为旧候选背书。
  private func admitManagedSelection(_ kind: String, client: IMKTextInput,
                                     stillValid: @escaping () -> Bool, perform: @escaping () -> Void) -> Bool {
    guard let snapshot = memoryDisplayedTargets[kind], let field = memoryTargetFields[snapshot.targetID],
      managedDisplayCurrent(kind, client: client), stillValid() else {
      _ = bridge.clearManagedMemory(); clearManagedMemoryDisplay(); return true
    }
    guard let token = personalInputDeferral.begin(now: ProcessInfo.processInfo.systemUptime) else { return true }
    let pending = PersonalDeferredOrigin(token: token, origin: snapshot, client: client,
      activation: voiceActivationGeneration, generation: localSelectionGeneration, epoch: personalization.epoch,
      schemaID: bridge.schemaID, code: latestComposing, selection: client.selectedRange(), marked: client.markedRange(), managedField: field)
    personalDeferredOrigin = pending
    let intent = UUID(), generation = memoryViewGeneration
    memorySelectionIntent = intent
    DispatchQueue.main.asyncAfter(deadline: .now() + InputiaPersonalInputDeferral<NSEvent>.timeout) { [weak self] in
      guard let self, self.personalInputDeferral.expired(token, now: ProcessInfo.processInfo.systemUptime) else { return }
      self.voiceStatus = "输入框核验超时，已取消候选与排队输入"
      self.finishDeferredPersonalInput(token)
      self.memorySelectionIntent = nil
    }
    InputiaVoiceInputLauncher.targetBridge(.init(kind: "validate", target: snapshot.inputiaTarget, purpose: "shared_terms"),
      personalAdmissionDelivery: true) { [weak self] reply in
      guard let self, self.memorySelectionIntent == intent, self.personalInputDeferral.token == token else { return }
      self.memorySelectionIntent = nil
      guard self.memoryViewGeneration == generation, let reply,
        self.personalReplayProofIsCurrent(reply, pending: pending, composition: true),
        !self.personalInputDeferral.expired(token, now: ProcessInfo.processInfo.systemUptime),
        InputiaMemorySelectionAdmission.matches(expected: InputiaMemoryTarget(snapshot.inputiaTarget), field: field,
          returned: reply.target.map(InputiaMemoryTarget.init), server: reply.server_instance, returnedField: reply.field_instance,
          ready: reply.ready, deadline: reply.deadline, now: ProcessInfo.processInfo.systemUptime),
        self.memoryDisplayedTargets[kind] === snapshot,
        self.managedDisplayCurrent(kind, client: client), stillValid() else {
        self.voiceStatus = "输入框或候选已变化，已取消候选与排队输入"
        self.finishDeferredPersonalInput(token)
        _ = self.bridge.clearManagedMemory(); self.clearManagedMemoryDisplay(); return
      }
      guard let queued = self.personalInputDeferral.finish(token) else { return }
      self.personalDeferredOrigin = nil; self.personalRecoveryToken = nil
      perform()
      if self.personalBoundaryWait.isWaiting { self.personalBoundaryProof = reply }
      self.replayDeferredPersonalInput(queued, pending: pending, proof: reply)
    }
    return true
  }

  private func scheduleManagedRank(client: IMKTextInput) {
    guard !candidatePanelExpanded, let pool = bridge.personalCandidatePool(limit: 64) else { return }
    let texts = pool.candidates.map(\.text)
    guard !texts.isEmpty, texts.count <= 64 else { return }
    requestManagedMemory(.rank(texts), client: client) { [weak self, weak object = client as AnyObject] in
      guard let self, let client = object as? IMKTextInput, !self.candidatePanelExpanded else { return }
      self.syncHostState(with: self.bridge.latestOutcome)
      self.updateCandidateWindow(client: client)
      self.schedulePersonalization(client: client)
      _ = self.refreshHotwordPrefix(client: client)
    }
  }

  private func retireWordSpan(reason: String) {
    wordSpanPreparation = nil; wordSpanObservation = UUID()
    wordSpan.retire(reason: reason)
  }

  private func wordSpanOwner(_ client: IMKTextInput) -> String {
    voiceControllerID + ":" + String(describing: ObjectIdentifier(client as AnyObject))
  }

  private func wordSpanCaret(_ client: IMKTextInput) -> UInt64? {
    let range = client.selectedRange(), marked = client.markedRange()
    guard range.location != NSNotFound, range.location >= 0, range.length == 0,
      marked.location == NSNotFound || marked.length == 0 else { return nil }
    return UInt64(range.location)
  }

  private func wordSpanScopeCurrent(_ context: InputiaWordSpanContext, caret: UInt64) -> Bool {
    guard InputiaHost.activeInputController === self, isCurrentInputiaSourceSelected(),
      !IsSecureEventInputEnabled(), InputiaPermissionLifecycle.shared.permits(permissionEpoch),
      context.activation == voiceActivationGeneration, context.field == memoryFieldID,
      memoryTargetFields[context.target.target_id] == context.field,
      bridge.latestOutcome.mode == "English", latestComposing.isEmpty, shiftEnglishComposition.isEmpty,
      let client = client(), wordSpanOwner(client) == context.owner, wordSpanCaret(client) == caret,
      let snapshot = voiceTargetSnapshots[context.target.target_id],
      snapshot.isCurrentForTypedOrigin(client: client, controllerID: voiceControllerID,
        activationGeneration: voiceActivationGeneration) else { return false }
    // 此处只核宿主范围；真实 AX 字段、正文及 anchors 必须由后台 checkpoint 读回。
    return true
  }

  private func prepareWordSpan(client: IMKTextInput, retryAfterHandshake: Bool = true) {
    guard wordSpanPreparation == nil, !wordSpan.isBusy, InputiaHost.activeInputController === self,
      bridge.latestOutcome.mode == "English", latestComposing.isEmpty, shiftEnglishComposition.isEmpty,
      recallCandidates.isEmpty, englishCompletionPrefix.isEmpty, wordSpanCaret(client) != nil else { return }
    let pending = UUID(); wordSpanPreparation = pending
    let selection = localSelectionGeneration, activation = voiceActivationGeneration
    InputiaVoiceBridge.shared.prepare { [weak self] result in
      guard let self else { return }
      guard self.wordSpanPreparation == pending else {
        // 首次认证握手的真实屏障会取消准备；只在期间完全没有输入/失活时重试一次新许可。
        if retryAfterHandshake, case .success = result, self.localSelectionGeneration == selection,
          self.voiceActivationGeneration == activation { self.prepareWordSpan(client: client, retryAfterHandshake: false) }
        return
      }
      guard case .success(let policy) = result, self.localSelectionGeneration == selection,
        self.voiceActivationGeneration == activation else { self.wordSpanPreparation = nil; return }
      self.prepareUnifiedVoiceTarget(client: client) { [weak self] target in
        guard let self else { if let target { InputiaVoiceInputLauncher.releaseTarget(target.target_id) }; return }
        guard self.wordSpanPreparation == pending, self.localSelectionGeneration == selection,
          self.voiceActivationGeneration == activation, let target,
          let field = self.memoryTargetFields[target.target_id], let caret = self.wordSpanCaret(client) else {
          if self.wordSpanPreparation == pending { self.wordSpanPreparation = nil }
          if let target { InputiaVoiceInputLauncher.releaseTarget(target.target_id); self.removeRetiredVoiceTarget(target.target_id) }
          return
        }
        self.wordSpanPreparation = nil
        self.wordSpan.prepare(.init(target: .init(target), policy: policy, field: field,
          owner: self.wordSpanOwner(client), activation: activation, caret: caret))
      }
    }
  }

  private func wordSpanEdit(_ event: NSEvent, client: IMKTextInput) -> InputiaWordSpanEdit? {
    guard bridge.latestOutcome.mode == "English", latestComposing.isEmpty, shiftEnglishComposition.isEmpty,
      recallCandidates.isEmpty, personalPredictions.isEmpty, !candidatePanelExpanded,
      event.modifierFlags.intersection([.command, .control, .option]).isEmpty,
      wordSpanCaret(client) != nil else { return nil }
    if event.keyCode == 51 { return .tailBackspace(1) }
    guard ![keyCodeTab, keyCodeReturn, keyCodeKeypadEnter, keyCodeEscape].contains(event.keyCode),
      let text = event.characters, !text.isEmpty, text.utf8.count <= 64,
      text.utf8.allSatisfy({ (32...126).contains($0) }) else { return nil }
    return .append(text)
  }

  /// 基础按键始终先走原路径。系统透传编辑在下一主线程轮次核对光标，无法确证即停止学习。
  private func handleWordSpanKeyDown(_ event: NSEvent, client: IMKTextInput) -> Bool {
    let edit = wordSpanEdit(event, client: client), before = wordSpanCaret(client)
    let hadPermit = wordSpan.hasPermit
    if !hadPermit { retireWordSpan(reason: "input_before_permit") }
    let observation = UUID(); wordSpanObservation = observation
    let handled = handleKeyDown(event, client: client)
    guard let edit, let before else { return handled }
    let observe = { [weak self] in
      guard let self, self.wordSpanObservation == observation else { return }
      guard let after = self.wordSpanCaret(client) else { self.retireWordSpan(reason: "caret_unobservable"); return }
      if hadPermit { self.wordSpan.observed(edit, before: before, after: after) }
      else if case .append(let text) = edit, let last = text.utf8.last,
        !InputiaWordSpanState.wordUnit(last), after == before + UInt64(text.utf8.count) {
        self.prepareWordSpan(client: client)
      }
    }
    if handled { observe() }
    else { DispatchQueue.main.async(execute: observe) }
    return handled
  }

  private func prepareUnifiedVoiceTarget(client: IMKTextInput?, completion: @escaping (InputiaVoiceTarget?) -> Void) {
    pruneVoiceTargetSnapshots()
    synchronizePermissionEpoch()
    let initialReason: String
    if !InputiaPermissionLifecycle.shared.isReady { initialReason = "policy" }
    else if IsSecureEventInputEnabled() { initialReason = "secure_input_enabled" }
    else if client == nil { initialReason = "missing_client" }
    else if client?.bundleIdentifier() == nil { initialReason = "missing_bundle" }
    else if client.map({ InputiaVoiceTargetSnapshot.validRange($0.selectedRange()) == nil }) == true { initialReason = "invalid_selection" }
    else { initialReason = "ok" }
    InputiaPersonalizationDiagnostics.record("capture_guard", initialReason)
    guard InputiaPermissionLifecycle.shared.isReady, !IsSecureEventInputEnabled(), let client,
      let bundle = client.bundleIdentifier(), let selection = InputiaVoiceTargetSnapshot.validRange(client.selectedRange()) else { completion(nil); return }
    let epoch = permissionEpoch
    let activation = voiceActivationGeneration
    let composition = localCompositionGeneration
    let localSelection = localSelectionGeneration
    let draft = InputiaVoiceTarget(target_id: UUID().uuidString, host_instance: InputiaVoiceServiceConnection.processInstance,
      controller_id: voiceControllerID, activation_generation: activation, field_id: nil,
      selection_generation: localSelection, composition_generation: composition, source_app: bundle)
    InputiaVoiceInputLauncher.targetBridge(.init(kind: "capture", draft: draft)) { [weak self] reply in
      guard let self else { completion(nil); return }
      let current = self.client()
      let captureReason: String
      if reply == nil { captureReason = "no_reply" }
      else if reply?.target == nil { captureReason = "missing_target" }
      else if reply?.target?.host_instance != draft.host_instance || reply?.target?.controller_id != draft.controller_id
        || reply?.target?.activation_generation != activation || reply?.target?.composition_generation != composition
        || reply?.target?.source_app != bundle { captureReason = "target_identity" }
      else if InputiaHost.activeInputController !== self || !isCurrentInputiaSourceSelected() { captureReason = "not_active" }
      else if !InputiaPermissionLifecycle.shared.permits(epoch) { captureReason = "policy" }
      else if self.voiceActivationGeneration != activation { captureReason = "activation" }
      else if self.localCompositionGeneration != composition { captureReason = "composition" }
      else if self.localSelectionGeneration != localSelection { captureReason = "selection_generation" }
      else if current == nil { captureReason = "missing_client" }
      else if current.map({ ObjectIdentifier($0 as AnyObject) != ObjectIdentifier(client as AnyObject) }) == true { captureReason = "client_identity" }
      else if current?.selectedRange() != selection { captureReason = "selection_changed" }
      else if ProcessInfo.processInfo.systemUptime >= (reply?.deadline ?? 0) { captureReason = "expired" }
      else { captureReason = "ok" }
      if captureReason != "ok" { InputiaPersonalizationDiagnostics.record("capture_reply", captureReason) }
      guard captureReason == "ok", let reply, let target = reply.target, let current else {
        if let id = reply?.target?.target_id { InputiaVoiceInputLauncher.releaseTarget(id) }
        completion(nil); return
      }
      guard target.field_id != nil else {
        InputiaPersonalizationDiagnostics.record("capture_reply", "history_only")
        completion(target); return
      }
      guard let remoteSelection = reply.selection, remoteSelection.location == selection.location,
        remoteSelection.length == selection.length else {
        InputiaPersonalizationDiagnostics.record("capture_reply", "remote_selection_mismatch")
        InputiaVoiceInputLauncher.releaseTarget(target.target_id); completion(nil); return
      }
      InputiaPersonalizationDiagnostics.record("capture_reply", "ok")
      let snapshot = InputiaVoiceTargetSnapshot.Snapshot(target: target, client: current, selection: selection,
        compositionGeneration: composition, localSelectionGeneration: localSelection, deadline: reply.deadline, permissionEpoch: epoch)
      if let field = reply.field_instance {
        let identity = reply.server_instance + ":" + field
        if let previous = self.memoryFieldID, previous != identity {
          _ = self.bridge.clearManagedMemory()
          self.clearManagedMemoryDisplay()
        }
        self.memoryFieldID = identity
        self.memoryTargetFields[target.target_id] = identity
      }
      self.voiceTargetSnapshots[target.target_id] = snapshot
      self.voicePermissionEpochs[target.target_id] = epoch
      completion(target)
    }
  }

  private func startUnifiedVoice(client: IMKTextInput?) {
    prepareUnifiedVoiceTarget(client: client) { [weak self] target in
      guard let self else { if let target { InputiaVoiceInputLauncher.releaseTarget(target.target_id) }; return }
      InputiaVoiceInputLauncher.triggerUnifiedVoice(target: target, deliver: { [weak self] delivery, acknowledge in
        guard let self else { acknowledge("pending_target"); return }
        self.deliverUnifiedVoice(delivery, acknowledge: acknowledge)
      }) { [weak self] message in self?.voiceStatus = message }
    }
  }

  private func deliverUnifiedVoice(_ delivery: InputiaVoiceDelivery, acknowledge: @escaping (String) -> Void) {
    guard let snapshot = voiceTargetSnapshots[delivery.target_id], let nonce = delivery.dispatchNonce, !nonce.isEmpty,
      localCompositionGeneration == snapshot.compositionGeneration, localSelectionGeneration == snapshot.localSelectionGeneration,
      InputiaHost.activeInputController === self, isCurrentInputiaSourceSelected() else { acknowledge("pending_target"); return }
    synchronizePermissionEpoch()
    guard let epoch = voicePermissionEpochs[delivery.target_id], InputiaPermissionLifecycle.shared.permits(epoch),
      let snapshot = voiceTargetSnapshots[delivery.target_id] else {
      acknowledge("pending_target")
      return
    }
    defer { voiceTargetSnapshots.removeValue(forKey: delivery.target_id); voicePermissionEpochs.removeValue(forKey: delivery.target_id); InputiaVoiceInputLauncher.releaseTarget(delivery.target_id) }
    guard !attemptedVoiceOutputOperations.contains(delivery.operation_id) else {
      acknowledge("uncertain")
      return
    }
    let decision = snapshot.dispatchDecision(
      delivery: delivery,
      client: client(),
      controllerID: voiceControllerID,
      activationGeneration: voiceActivationGeneration,
      latestComposing: latestComposing,
      isSensitiveApp: { [weak self] bundleID, windowTitle in
        self?.bridge.isSensitiveApp(bundleId: bundleID, windowTitle: windowTitle) ?? true
      },
      windowTitle: { [weak self] bundleID in
        self?.checkedWindowTitle(forBundleId: bundleID) ?? .unavailable
      }
    )
    guard case .dispatch(let client) = decision else {
      acknowledge("pending_target")
      return
    }
    guard InputiaVoiceTargetSnapshot.isWithinDispatchDeadline(delivery) else {
      acknowledge("pending_target")
      return
    }
    guard InputiaPermissionLifecycle.shared.permits(epoch) else { acknowledge("pending_target"); return }
    attemptedVoiceOutputOperations.insert(delivery.operation_id)
    personalization.reset(); personalPhrase.reset(); personalUndo = nil
    client.insertText(delivery.text, replacementRange: emptyReplacementRange)
    acknowledge("dispatched")
  }

  private func voiceTargetCaptureStatus(reason: String) -> String {
    switch reason {
    case "accessibility_permission_required":
      return "候选输入法需要辅助功能权限才能核对原输入框；转写仍会保存到历史记录。"
    case "secure_input_enabled", "secure_text_field":
      return "当前输入框受安全输入保护，未启动录音。"
    case "field_unobservable", "selection_unobservable", "field_observer_unavailable", "unsupported_focused_role":
      return "当前输入框暂时无法可靠观察；转写会保存到历史记录。"
    case "focused_application_mismatch":
      return "当前输入目标与前台应用不一致，未启动录音。"
    default:
      return "当前无法核对原输入框；转写会保存到历史记录。"
    }
  }

  private func pruneVoiceTargetSnapshots() {
    let now = ProcessInfo.processInfo.systemUptime
    let expired = voiceTargetSnapshots.filter { now - $0.value.createdAt > 120 }.map { $0.key }
    for id in expired { removeRetiredVoiceTarget(id); InputiaVoiceInputLauncher.releaseTarget(id) }
    voicePermissionEpochs = voicePermissionEpochs.filter { voiceTargetSnapshots[$0.key] != nil }
    if voiceTargetSnapshots.count > 16 {
      voiceTargetSnapshots.removeAll()
    }
    if attemptedVoiceOutputOperations.count > 64 {
      attemptedVoiceOutputOperations.removeAll(keepingCapacity: true)
    }
  }
  #endif

  private func handleCandidateNavigation(
    _ navigation: InputiaCandidateNavigation,
    client: IMKTextInput
  ) -> Bool {
    #if INPUTIA_PAIRED_BUILD
    clearSharedChineseCandidates()
    #endif
    guard !latestComposing.isEmpty else {
      return false
    }
    cancelShiftInputModeGesture(reason: "candidateNavigation")
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
    #if INPUTIA_PAIRED_BUILD
    personalRefreshGeneration &+= 1
    personalization.invalidateView()
    clearSharedChineseCandidates()
    #endif
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
    #if INPUTIA_PAIRED_BUILD
    personalRefreshGeneration &+= 1
    personalization.invalidateView()
    clearSharedChineseCandidates()
    #endif
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
    #if INPUTIA_PAIRED_BUILD
    if !personalCandidates.isEmpty, personalCode == latestComposing {
      expandedCandidateEntries = personalCandidates.prefix(targetCount).enumerated().map {
        InputiaExpandedCandidateEntry(text: $0.element.text, page: 0, pageIndex: $0.offset,
          candidateID: $0.element.id, originalRank: $0.element.base_rank)
      }
      expandedCandidates = expandedCandidateEntries.map(\.text)
      clampExpandedActiveRow()
      return
    }
    #endif
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
    #if INPUTIA_PAIRED_BUILD
    if let id = entry.candidateID {
      guard InputiaPersonalContext.retainsPersonalIdentity(id, available: personalCandidates.map(\.id)),
        let client, let candidate = personalCandidates.first(where: { $0.id == id && $0.text == entry.text }),
        personalCode == latestComposing else { return true }
      return choosePersonal(candidate, explicit: true, client: client)
    }
    #endif
    guard moveBridgeToPage(entry.page) else {
      return false
    }
    guard let client else { return false }
    return chooseNativeWithLearning(index: entry.pageIndex, explicit: true, client: client)
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
    #if INPUTIA_PAIRED_BUILD
    // 统一产品由主程序注册可配置的全局快捷键，避免同一组合弹出两套窗口。
    return false
    #else
    return InputiaShortcutClassifier.isClipboardRecall(
      charactersIgnoringModifiers: event.charactersIgnoringModifiers,
      modifiers: modifiers
    )
    #endif
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
    guard context.windowTitleAvailable, bridge.shouldReadClipboard(bundleId: context.bundleId, windowTitle: context.windowTitle) else {
      inputiaDebugLog("clipboardRecallSkipped reason=privacy")
      clearClipboardRecall()
      return false
    }
    learnAndClearEnglishCompletion(client: client)

    #if INPUTIA_PAIRED_BUILD
    if bridge.managedMemoryEnabled {
      clearClipboardRecall()
      requestManagedMemory(.clipboard(9), client: client) { [weak self, weak object = client as AnyObject] in
        guard let self, let client = object as? IMKTextInput else { return }
        _ = self.presentClipboardRecall(self.bridge.clipboardCandidates(limit: 9), client: client)
      }
      return true
    }
    #endif

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

    return presentClipboardRecall(bridge.clipboardCandidates(limit: 9), client: client)
  }

  private func presentClipboardRecall(_ candidates: [String], client: IMKTextInput) -> Bool {
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
    #if INPUTIA_PAIRED_BUILD
    if bridge.managedMemoryEnabled {
      return admitManagedSelection("clipboard", client: client, stillValid: { [weak self] in
        self?.recallCandidates.indices.contains(index) == true && self?.recallCandidates[index] == text
          && self?.bridge.clipboardCandidates(limit: 9).contains(text) == true
      }) { [weak self] in
        client.insertText(text, replacementRange: emptyReplacementRange)
        self?.clearClipboardRecall()
      }
    }
    #endif
    client.insertText(text, replacementRange: emptyReplacementRange)
    inputiaDebugLog("clipboardRecallCommit index=\(index)")
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

  private func refreshEnglishCompletions(client: IMKTextInput, includeShared: Bool = false, fetchManaged: Bool = true) {
    guard englishCompletionPrefix.count >= 2 else {
      hideEnglishCompletionCandidates()
      return
    }
    #if INPUTIA_PAIRED_BUILD
    if fetchManaged, bridge.managedMemoryEnabled {
      let prefix = englishCompletionPrefix
      requestManagedMemory(.englishCompletion(prefix, 5), client: client) { [weak self, weak object = client as AnyObject] in
        guard let self, let client = object as? IMKTextInput, self.englishCompletionPrefix == prefix else { return }
        self.refreshEnglishCompletions(client: client, includeShared: includeShared, fetchManaged: false)
      }
    }
    #endif
    var candidates = bridge.completionCandidates(prefix: englishCompletionPrefix, limit: 5)
      .filter { completionSuffix(for: $0) != nil }
    #if INPUTIA_PAIRED_BUILD
    let explicitSnapshot = InputiaExplicitHotwords.shared.snapshot()
    explicitEnglishCandidates = [:]
    candidates.removeAll { explicitSnapshot.words.contains($0) }
    sharedEnglishCandidates = [:]
    if includeShared, let shared = liveSharedTerms(client: client) {
      let words = shared.englishCandidates(prefix: englishCompletionPrefix).filter { !shared.explicitTerms.contains($0) && completionSuffix(for: $0) != nil }
      var seen = Set<String>()
      candidates = Array((words + candidates).filter { seen.insert($0).inserted }.prefix(5))
      for word in words where candidates.contains(word) { sharedEnglishCandidates[word] = shared.identity }
    } else if includeShared {
      InputiaSharedTermsMemory.shared.clear()
    } else {
      scheduleSharedEnglishRefresh(client: client)
    }
    if explicitInputAllowed(client: client) {
      let words = InputiaHotwordPrefix.candidates(explicitSnapshot.words, code: englishCompletionPrefix)
        .filter { $0.lowercased().hasPrefix(englishCompletionPrefix.lowercased()) && completionSuffix(for: $0) != nil }
      var seen = Set<String>()
      candidates = Array((words + candidates).filter { seen.insert($0).inserted }.prefix(5))
      for word in words where candidates.contains(word) { explicitEnglishCandidates[word] = (explicitSnapshot, explicitSelectionContext(client: client, snapshot: explicitSnapshot)) }
    }
    #endif
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
    englishCompletionRect = inputRect
    InputiaHost.candidatePanel?.show(candidates: candidates, near: inputRect)
    #if INPUTIA_PAIRED_BUILD
    prepareMemoryCommit(candidates: candidates, prefix: englishCompletionPrefix, client: client)
    #endif
    inputiaDebugLog("englishCompletionShown count=\(candidates.count)")
  }

  private func commitFirstEnglishCompletion(client: IMKTextInput) -> Bool {
    guard let candidate = englishCompletionCandidates.first else {
      return false
    }
    return commitEnglishCompletion(candidate, client: client)
  }

  private func commitEnglishCompletion(_ candidate: String, client: IMKTextInput, memoryValidated: Bool = false) -> Bool {
    #if INPUTIA_PAIRED_BUILD
    if let explicit = explicitEnglishCandidates[candidate] {
      let expected = explicit.context
      let current = InputiaExplicitHotwords.shared.snapshot()
      guard expected.admits(current: explicitSelectionContext(client: client, snapshot: current), word: candidate,
        snapshot: current, secure: IsSecureEventInputEnabled(), active: explicitInputAllowed(client: client)),
        bridge.latestOutcome.mode == "English", englishCompletionPrefix.count == 3,
        candidate.lowercased().hasPrefix(englishCompletionPrefix.lowercased()),
        let suffix = completionSuffix(for: candidate), !suffix.isEmpty else {
        clearEnglishCompletion(); return true
      }
      client.insertText(suffix, replacementRange: emptyReplacementRange)
      clearEnglishCompletion()
      return true
    }
    if let identity = sharedEnglishCandidates[candidate] {
      return enqueueSharedEnglishSelection(candidate, identity: identity, client: client)
    }
    #endif
    #if INPUTIA_PAIRED_BUILD
    if bridge.managedMemoryEnabled, !memoryValidated {
      let prefix = englishCompletionPrefix
      return admitManagedSelection("english_completion", client: client, stillValid: { [weak self] in
        self?.englishCompletionPrefix == prefix && self?.bridge.completionCandidates(prefix: prefix, limit: 5).contains(candidate) == true
      }) { [weak self] in _ = self?.commitEnglishCompletion(candidate, client: client, memoryValidated: true) }
    }
    #endif
    guard let suffix = completionSuffix(for: candidate), !suffix.isEmpty else {
      clearEnglishCompletion()
      return false
    }
    #if INPUTIA_PAIRED_BUILD
    let memoryConfirmation = takeMemoryConfirmation(text: suffix, replacement: client.selectedRange(), client: client)
    let origin = typedOriginBeforeInsertion(client)
    let typedStart = client.selectedRange().location
    #endif
    client.insertText(suffix, replacementRange: emptyReplacementRange)
    #if INPUTIA_PAIRED_BUILD
    memoryConfirmation?()
    recordTypedCommit(suffix, client: client, start: typedStart, origin: origin)
    #endif
    let context = appContext(for: client)
    if context.windowTitleAvailable {
      _ = bridge.learnTyped(text: candidate, bundleId: context.bundleId, windowTitle: context.windowTitle)
    }
    inputiaDebugLog("englishCompletionCommit")
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
      if context.windowTitleAvailable {
        _ = bridge.learnTyped(text: word, bundleId: context.bundleId, windowTitle: context.windowTitle)
      }
      inputiaDebugLog("englishWordLearned")
    }
    clearEnglishCompletion()
  }

  private func hideEnglishCompletionCandidates() {
    #if INPUTIA_PAIRED_BUILD
    sharedEnglishCandidates = [:]
    #endif
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
    #if INPUTIA_PAIRED_BUILD
    explicitEnglishCandidates = [:]
    #endif
    englishCompletionPrefix = ""
    hideEnglishCompletionCandidates()
  }

  private func clearShiftEnglishComposition(client: IMKTextInput?) {
    shiftEnglishComposition = ""
    if let client { clearMarkedText(client) }
    latestComposing = ""
    latestCandidates = []
    InputiaHost.candidatePanel?.hide()
  }

  private func commitShiftEnglishComposition(client: IMKTextInput) {
    let text = shiftEnglishComposition
    guard !text.isEmpty else { return }
    clearShiftEnglishComposition(client: client)
    client.insertText(text, replacementRange: emptyReplacementRange)
    inputiaDebugLog("shiftEnglishCompositionCommit text=\(text)")
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
    #if INPUTIA_PAIRED_BUILD
    clearHotwordOverlay()
    explicitEnglishCandidates = [:]
    discardTypedCompositionOrigin()
    #endif
    shiftEnglishComposition = ""
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
    chineseCandidateRect = inputRect
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
    guard context.windowTitleAvailable else {
      _ = bridge.setUnverifiedAppContext(bundleId: context.bundleId)
      pushedAppContext = nil
      return
    }
    inputiaDebugLog("contextRefreshed")
    if let pushedAppContext, pushedAppContext != context {
      resetShiftInputModeSession(reason: "contextChanged")
    }
    guard pushedAppContext != context else {
      return
    }
    _ = bridge.setAppContext(bundleId: context.bundleId, windowTitle: context.windowTitle)
    pushedAppContext = context
  }

  private func shouldUseSecureDirectMode(_ client: IMKTextInput) -> Bool {
    reloadSettingsIfDue(client: client)
    let context = appContext(for: client)
    guard IsSecureEventInputEnabled() || InputiaSecureDirectPolicy.shouldUseSecureDirectMode(context: context) else {
      return false
    }
    clearInputState(client: client)
    inputiaDebugLog("secureDirectPassthrough")
    if context.windowTitleAvailable && pushedAppContext != context {
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
      now.timeIntervalSince(cachedAppContextTime) < (cachedAppContext.windowTitleAvailable ? appContextRefreshInterval : 0.1)
    {
      return cachedAppContext
    }

    let context: InputiaAppContext
    switch checkedWindowTitle(forBundleId: bundleId) {
    case .ready(let title): context = InputiaAppContext(bundleId: bundleId, windowTitle: title)
    case .unavailable: context = InputiaAppContext(bundleId: bundleId, windowTitle: nil, windowTitleAvailable: false)
    }
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

  private func checkedWindowTitle(forBundleId bundleId: String) -> InputiaWindowTitleQuery.Result { .unavailable }

}

@main
struct InputiaInputMethodApp {
  private static var server: IMKServer?
  private static var appDelegate: InputiaApplicationDelegate?

  static func main() {
    autoreleasepool {
      InputiaStartupMaintenance.requireNormalStart()
      // 候选编译身份必须在创建IMK连接、设置窗口或诊断会话前与包身份一致。
      _ = InputiaProfile.current
      #if INPUTIA_PAIRED_BUILD
      #if INPUTIA_RELEASE_PAIR_V2
      guard InputiaProfile.current.installation != nil else {
        NSLog("Inputia release installation receipt missing")
        exit(78)
      }
      #else
      guard InputiaProfile.current.isCandidate,
            InputiaProfile.current.runID == InputiaEmbeddedPairTrust.runID else {
        NSLog("Inputia embedded pair trust does not match candidate profile")
        exit(78)
      }
      #endif
      InputiaMemoryBarrier.invalidate = {
        InputiaVoiceBridge.shared.retirePending()
        InputiaRustBridge.invalidateManagedMemory()
        for controller in InputiaHost.inputControllers.allObjects { controller.clearManagedMemoryDisplay() }
        InputiaHost.candidatePanel?.hide()
      }
      InputiaMemoryBarrier.clear = { policy in
        InputiaVoiceBridge.shared.retirePending()
        try InputiaRustBridge.applyManagedMemoryPolicy(policy)
        for controller in InputiaHost.inputControllers.allObjects { controller.clearManagedMemoryDisplay() }
        InputiaHost.candidatePanel?.hide()
      }
      #endif
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
      #if INPUTIA_PAIRED_BUILD
      InputiaSharedTermsMemory.shared.didClear = {
        InputiaHost.activeInputController?.clearSharedEnglishCandidates()
      }
      InputiaVoiceInputLauncher.didReleaseTarget = { id in
        for controller in InputiaHost.inputControllers.allObjects { controller.removeRetiredVoiceTarget(id) }
      }
      InputiaPermissionLifecycle.shared.configureProbe { InputiaVoiceInputLauncher.probeServicePermission() }
      InputiaPermissionLifecycle.shared.start(root: InputiaProfile.current.root.deletingLastPathComponent()) { completed in
        for controller in InputiaHost.inputControllers.allObjects { controller.synchronizePermissionEpoch() }
        InputiaHost.removeGlobalMonitors()
        let epoch = InputiaPermissionLifecycle.shared.epoch
        InputiaVoiceInputLauncher.invalidatePermissionWork {
          defer { completed() }
          guard InputiaPermissionLifecycle.shared.permits(epoch) else { return }
          InputiaHost.installGlobalMonitors()
          InputiaVoiceInputLauncher.ensureUnifiedServiceReady()
          InputiaVoiceInputLauncher.startShortcutListening(targetProvider: {
            InputiaHost.activeInputController?.shortcutRegistrationTarget()
          }, sharedTermsReceiver: { terms, ticket in
            InputiaHost.activeInputController?.acceptSharedTerms(terms, ticket: ticket)
          }, acceptStart: { trigger, completion in
            guard let controller = InputiaHost.activeInputController else { completion(false); return }
            controller.acceptUnifiedShortcut(trigger, completion: completion)
          })
        }
      }
      #endif
      #if !INPUTIA_PAIRED_BUILD
      InputiaHost.installGlobalMonitors()
      #endif
      NSLog("Inputia baseline IMK server started: bundle=\(resolvedBundleIdentifier), connection=\(resolvedConnectionName)")

      #if INPUTIA_PAIRED_BUILD
      InputiaExplicitHotwords.shared.didChange = { InputiaHost.activeInputController?.refreshExplicitHotwords() }
      InputiaExplicitHotwords.shared.start(file: InputiaProfile.current.handyRoot.appendingPathComponent("settings_store.json"))
      #endif
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
    case "--explicit-hotwords-self-check":
      #if INPUTIA_PAIRED_BUILD
      let words = InputiaExplicitHotwords.read(file: InputiaProfile.current.handyRoot.appendingPathComponent("settings_store.json"))
      let matches = InputiaHotwordPrefix.candidates(words, code: "lll").count
      print("explicit_hotwords loaded_count=\(words.count) prefix_matches=\(matches) voice_snapshot_required=false")
      return true
      #else
      fputs("explicit_hotwords unavailable: paired build required\n", stderr)
      exit(2)
      #endif
    case "--unified-runtime-self-check":
      InputiaRuntimeDiagnostics.run()
      return true
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
    let shiftGestureChecks = InputiaShortcutClassifier.shiftInputModeGestureSelfCheckResults()
    let shiftGestureSelfCheck = shiftGestureChecks.allSatisfy { $0.1 }

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
      && shiftGestureSelfCheck

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
    for (name, result) in shiftGestureChecks {
      print("\(name)=\(result)")
    }
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
