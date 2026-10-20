#!/usr/bin/env python3
"""无 GUI 跨状态回归：真实宿主调度/清理 + 真实 Personalization，仅替换原生边界与传输。"""

import argparse
from pathlib import Path
import subprocess
import tempfile


root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--main-source", type=Path)
parser.add_argument("--personalization-source", type=Path)
parser.add_argument("--scenario", choices=["lifecycle", "cleanup", "latency-policy"], default="lifecycle")
args = parser.parse_args()
source = (args.main_source or root / "Sources/InputiaInputMethod/main.swift").read_text()


def declaration(marker: str) -> str:
    start = source.index(marker)
    if source[:start].endswith("  @discardableResult\n"):
        start -= len("  @discardableResult\n")
    brace = source.index("{", start)
    depth, end = 1, brace + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end].replace("private ", "")


markers = [
    "  private struct PredictionCommitTrigger",
    "  private struct PersonalSelection",
    "  private struct PersonalUndo",
    "  private struct PersonalDeferredOrigin",
    "  private var ownedCandidatePanel:",
    "  private func predictionTriggerForSpace(",
    "  private func closePredictionPresentation(",
    "  private func retirePredictionPresentation()",
    "  private func currentPredictionToken(",
    "  private func beginPredictionAfterCommit(",
    "  private func schedulePersonalization(",
    "  private func clearPersonalDisplay()",
    "  private func apply(",
    "  private func syncHostState(",
    "  private func insertCommittedText(",
    "  private func releaseConfirmedFieldAnchor(",
    "  private func retainConfirmedFieldAnchor()",
    "  private func clearConfirmedFieldContext()",
    "  private func invalidateOwnedContext(",
    "  private func confirmedFieldScope(",
    "  private func recordConfirmedFieldCommit(",
    "  private func observeConfirmedFieldInput(",
    "  private func recordPersonalCommit(",
    "  private func observePersonalKey(",
    "  private func isPersonalCandidateNavigation(",
    "  private func isPersonalCompositionBackspace(",
    "  private func deferredPersonalScopeMatches(",
    "  private func finishDeferredPersonalInput(",
    "  private func recoverDeferredPersonalInput(",
    "  private func personalReplayProofIsCurrent(",
    "  private func replayDeferredPersonalInput(",
    "  private func recordTypedCommit(",
    "  private func typedOriginBeforeInsertion(",
    "  private func refreshTypedCompositionOrigin(",
    "  private func releaseTypedOriginIfUnowned(",
    "  private func discardPreparedVoiceTarget()",
    "  func shortcutRegistrationTarget()",
    "  private func updateCandidateWindow(",
    "  func synchronizePermissionEpoch()",
]
methods = "\n".join(declaration(marker) for marker in markers)
has_prediction_keys = "  private func predictionSelectionIndex(" in source
if has_prediction_keys:
    for marker in ["  private func predictionSelectionIndex(", "  private func handlePersonalCandidateKey("]:
        methods += "\n" + declaration(marker)
    if "handlePersonalCandidateKey(event, client: client)" not in declaration("  private func handleKeyDown("):
        raise AssertionError("真实handleKeyDown必须调用同一推荐按键入口")
has_accept_context = "var previous: String? = nil" in declaration("  private struct PersonalSelection")
if args.main_source is None and not has_accept_context:
    raise AssertionError("当前生产源码必须包含实际接受推荐的上下文传递，不能跳过该回归")
if has_accept_context:
    for marker in [
        "  private func acceptPersonalPrediction(",
        "  private func acceptAdmittedPersonalCandidate(",
    ]:
        methods += "\n" + declaration(marker)
for marker in [
    "  private struct PredictionQueryContext",
    "  private func invalidatePredictionTarget(",
    "  private func schedulePredictionTarget(",
    "  private func resumePredictionAfterPreCapture()",
]:
    if marker in source:
        methods += "\n" + declaration(marker)
cleanup_start = source.index("  @discardableResult\n  private func clearManagedMemoryDisplay(")
cleanup_end = source.index("  private func observePersonalKey(", cleanup_start)
methods += "\n" + source[cleanup_start:cleanup_end].replace("private ", "")
boundary = declaration("    if event.type == .keyDown || (event.type == .flagsChanged && changesInput)")
methods += "\nfunc physicalInput(_ event: NSEvent) -> Bool {\nvar predictionTransitionKeptWindow = false\nlet changesInput = true\nobserveConfirmedFieldInput(event, client: live)\n" + boundary + "\nreturn false\n}\n"
expiry_binding = next(line.strip() for line in source.splitlines() if "bridge.memoryDidExpire =" in line)
methods += "\nfunc installExpiryCallback() {\n" + expiry_binding + "\n}\n"
capture_anchor_start = next(line.strip() for line in source.splitlines() if "let anchorAtCapture = confirmedFieldAnchor" in line)
capture_anchor_finish = next(line.strip() for line in source.splitlines() if "defer { if let anchorAtCapture" in line)
callbacks = declaration("    personalization.ownContextInvalidated =") + "\n" + declaration("    personalization.changed =") + "\n" + declaration("    personalization.policyChanged =")

fixture = r'''
import AppKit
setbuf(stdout, nil)
let keyCodeSpace: UInt16 = 49, keyCodeTab: UInt16 = 48, keyCodeEscape: UInt16 = 53
let keyCodeReturn: UInt16 = 36, keyCodeKeypadEnter: UInt16 = 76, keyCodeDelete: UInt16 = 51
let keyCodePageDown: UInt16 = 121, keyCodePageUp: UInt16 = 116
let emptyReplacementRange = NSRange(location: NSNotFound, length: 0)
func inputiaDebugLog(_ value: String) {}
struct InputiaVoiceTarget: Equatable {
  let target_id: String
  var host_instance = "fixture-host", controller_id = "fixture-controller"
  var activation_generation: UInt64 = 0
  var field_id: String? = "field-A"
  var selection_generation: UInt64 = 0, composition_generation: UInt64 = 0
  var source_app: String? = "fixture.app"
}
struct InputiaPersonalCandidate: Equatable { let id: String; let text: String; let base_rank: Int; let consumed_len: Int; var match_type: String? = nil }
struct InputiaPersonalPrediction: Equatable { let id: String; let text: String }
struct InputiaPersonalResult {
  let ordered_ids: [String]?; let predictions: [InputiaPersonalPrediction]?; let context_id: String?
  var admitted: Bool? = nil; var prediction_id: String? = nil
  var recalled_candidates: [InputiaPersonalCandidate]? = nil
}
struct InputiaPersonalReply {
  let server_instance: String; let enabled: Bool; let epoch: UInt64
  let result: InputiaPersonalResult?; let code: String?
}
struct InputiaPersonalCommand {
  let kind: String
  var target: InputiaVoiceTarget? = nil
  var learning_epoch: UInt64? = nil
  var input_code: String? = nil
  var schema_id: String? = nil
  var context: String? = nil
  var context_id: String? = nil
  var candidates: [InputiaPersonalCandidate]? = nil
  var limit: Int? = nil
  var event_id: String? = nil
  var text: String? = nil
  var previous: String? = nil
  var explicit_selection: Bool? = nil
  var original_rank: Int? = nil
  var operation: String? = nil
  var prediction_id: String? = nil
}
struct InputiaTargetBridgeCommand { let kind: String; let target: InputiaVoiceTarget; let purpose: String }
struct InputiaTargetBridgeReply {
  var target: InputiaVoiceTarget? = nil
  var ready = true, server_instance = "service"
  var permission_epoch: UInt64 = 1
  var deadline = ProcessInfo.processInfo.systemUptime + 0.75
  var field_instance: String? = nil
}
enum InputiaPersonalizationDiagnostics {
  static func record(_ phase: String, _ reason: String, flags: Int = 0, count: Int = 0) {}
}
enum InputiaVoiceInputLauncher {
  struct Pending { let command: InputiaPersonalCommand; let deadline: TimeInterval; let completion: (InputiaPersonalReply?) -> Void }
  static var pending: [Pending] = [], sent: [InputiaPersonalCommand] = []
  static var released: [String] = []
  static var targetReplies: [(InputiaTargetBridgeReply?) -> Void] = []
  static func personalization(_ command: InputiaPersonalCommand, deadline: TimeInterval,
    expectedServer: String? = nil, completion: @escaping (InputiaPersonalReply?) -> Void) {
    sent.append(command); pending.append(.init(command: command, deadline: deadline, completion: completion))
  }
  static var nativeLeases: [String: (field: InputiaConfirmedFieldContext.Field, expiresAt: TimeInterval)] = [:]
  static func releaseTarget(_ id: String) { released.append(id); nativeLeases.removeValue(forKey: id) }
  static func targetBridge(_ command: InputiaTargetBridgeCommand, personalAdmissionDelivery: Bool,
    completion: @escaping (InputiaTargetBridgeReply?) -> Void) { targetReplies.append(completion) }
}
final class Client {
  var selection = NSRange(location: 2, length: 0), marked = NSRange(location: 0, length: 2)
  var inserts = 0
  var selectionHook: (() -> Void)?
  var selectionReads = 0
  func selectedRange() -> NSRange {
    selectionReads += 1
    let previous = selection, hook = selectionHook; selectionHook = nil; hook?(); return previous
  }
  func markedRange() -> NSRange { marked }
  func bundleIdentifier() -> String? { "fixture.app" }
  func attributes(forCharacterIndex: Int, lineHeightRectangle: inout NSRect) {}
  func attributedSubstring(from: NSRange) -> NSAttributedString? { nil }
  func insertText(_ text: String, replacementRange: NSRange) {
    inserts += 1
    let start = replacementRange.location == NSNotFound ? selection.location : replacementRange.location
    selection = NSRange(location: start + text.utf16.count, length: 0); marked = emptyReplacementRange
  }
}
typealias IMKTextInput = Client
enum InputiaVoiceTargetSnapshot {
  final class Snapshot {
    let inputiaTarget: InputiaVoiceTarget
    var targetID: String { inputiaTarget.target_id }
    var controllerID: String { inputiaTarget.controller_id }
    var activationGeneration: UInt64 { inputiaTarget.activation_generation }
    var expiresAt = ProcessInfo.processInfo.systemUptime + 120
    var reusableForShortcut: Bool { valid && ProcessInfo.processInfo.systemUptime < expiresAt }
    var compositionGeneration: UInt64 = 0, localSelectionGeneration: UInt64 = 0
    var permissionEpoch = InputiaPermissionLifecycle.shared.epoch
    var confirmedField: InputiaConfirmedFieldContext.Field? = .init(server: "service", instance: "field-a")
    var valid = true
    init(_ target: InputiaVoiceTarget) { inputiaTarget = target }
    func isCurrentForTypedOrigin(client: Client?, controllerID: String, activationGeneration: UInt64) -> Bool {
      valid && client != nil && inputiaTarget.controller_id == controllerID
        && inputiaTarget.activation_generation == activationGeneration
    }
    func isCurrentForShortcut(client: Client?, controllerID: String, activationGeneration: UInt64,
      isSensitiveApp: (String, String?) -> Bool, windowTitle: (String) -> InputiaWindowTitleQuery.Result) -> Bool {
      isCurrentForTypedOrigin(client: client, controllerID: controllerID, activationGeneration: activationGeneration)
    }
  }
  static func validRange(_ range: NSRange) -> NSRange? { range.location == NSNotFound ? nil : range }
  static func shouldAttemptPreCapture(now: TimeInterval, retryDeadline: TimeInterval?) -> Bool { now >= (retryDeadline ?? 0) }
  static func preCaptureRetryDeadline(after failureTime: TimeInterval) -> TimeInterval { failureTime + 2 }
}
enum InputiaWindowTitleQuery { enum Result { case unavailable } }
enum InputiaTypedOriginLifetime {
  static func retain<T>(_ existing: T?, existingAllowed: Bool, candidate: T?, boundary: Bool) -> T? {
    boundary ? nil : (existingAllowed ? (existing ?? candidate) : candidate)
  }
}
final class InputiaTypedCapture {
  static let shared = InputiaTypedCapture()
  var writes = 0
  func resetSegment() {}
  func committed(text: String, draft: InputiaVoiceTarget, identity: String, start: Int, end: Int) { writes += 1 }
}
struct InputiaBridgeOutcome {
  var ok = true, consumed = true, mode = "Chinese", composing = "ni", candidates = ["你", "呢"]
  var commit: String? = nil
  var candidateIDs = ["rime:a", "rime:b"], page = 0
}
final class Bridge {
  var latestOutcome = InputiaBridgeOutcome(), schemaID = "luna"
  struct Pool { let candidates: [InputiaPersonalCandidate] }
  var clears = 0
  var memoryDidExpire: (() -> Void)?
  func clearManagedMemory() -> Bool { clears += 1; return true }
  func cancelManagedMemoryRequests() {}
  func escape() -> InputiaBridgeOutcome { .init(composing: "", candidates: [], candidateIDs: []) }
  func undoRecentNativeLearning() -> Bool { true }
  func personalCandidatePool(limit: Int) -> Pool? { Pool(candidates: []) }
}
final class InputiaCandidatePanel {
  var visible = true, shown = ["你", "呢"], hides = 0, pending = 0, replacements = 0
  var restorationSequence: UInt64 = 0, restorationToken: UInt64?
  func clearForRestoration() -> UInt64 {
    restorationSequence += 1; restorationToken = restorationSequence; shown = []; return restorationSequence
  }
  func isRestorationCurrent(_ token: UInt64) -> Bool { restorationToken == token }
  func hide() { restorationToken = nil; hides += 1; visible = false; shown = [] }
  func showPredictionPending(near: NSRect) { restorationToken = nil; pending += 1; visible = true; shown = [] }
  func showPredictionEmpty(near: NSRect) { restorationToken = nil; visible = true; shown = [] }
  func show(candidates: [String], near: NSRect, expanded: Bool = false,
    primaryCandidateCount: Int? = nil, activeRowIndex: Int = 0) {
    restorationToken = nil; replacements += 1; visible = true; shown = candidates
  }
}
enum InputiaHost {
  static var activeInputController: Controller?
  static var candidatePanel: InputiaCandidatePanel? = InputiaCandidatePanel()
}
final class InputiaPermissionLifecycle {
  static let shared = InputiaPermissionLifecycle()
  enum Status { case ready, unknown, maintenance }
  struct Snapshot { var status = Status.ready }
  var snapshot = Snapshot(), epoch: UInt64 = 1
  var isReady: Bool { snapshot.status == .ready }
  func matchesService(server: String, epoch: UInt64) -> Bool { server == "service" && epoch == 1 }
  func permits(_ value: UInt64) -> Bool { isReady && value == epoch }
}
final class InputiaTargetCaptureScope { func invalidate() {} }
final class InputiaSharedTermsMemory {
  static let shared = InputiaSharedTermsMemory()
  func clear() {}
}
enum InputiaSecureDirectPolicy { static func shouldUseSecureDirectMode(context: Bool) -> Bool { context } }
var secure = false, sourceSelected = true
func IsSecureEventInputEnabled() -> Bool { secure }
func isCurrentInputiaSourceSelected() -> Bool { sourceSelected }
enum InputiaHostTextPolicy {
  static func commitReplacementRange(previousComposing: String, markedRange: NSRange) -> NSRange { markedRange }
  static func shouldContinueMarkedTextAfterCommit(committedText: String?, nextComposing: String) -> Bool { !nextComposing.isEmpty }
  static func shouldClearMarkedText(previousComposing: String, nextComposing: String, committedText: String?) -> Bool { !previousComposing.isEmpty && nextComposing.isEmpty }
  static func candidatesForPanel(composing: String, candidates: [String]) -> [String] { candidates }
}
struct Entry { var candidateID: String? = "old-id" }
struct Resettable { mutating func cancel() {} }
final class Controller {
  let bridge = Bridge(), personalization = InputiaPersonalization()
  var live = Client()
  var candidateDisplayCleanupDepth = 0, predictionHandoffDepth = 0
  let targetCaptureScope = InputiaTargetCaptureScope()
  var predictionSelectionHandoff: UInt64?, predictionDisplayedAt: TimeInterval?
  var personalInputReplayDepth = 0, personalReplayKeepsTypedOrigin = false
  var confirmedFieldAnchor: InputiaVoiceTargetSnapshot.Snapshot?
  var confirmedFieldAnchorExpiry: DispatchWorkItem?
  var confirmedFieldContext = InputiaConfirmedFieldContext()
  var predictionPresentation = InputiaPredictionPresentation(), predictionInputGeneration: UInt64 = 1
  var predictionExpiry: DispatchWorkItem?, predictionClient: ObjectIdentifier?, predictionSelection: NSRange?
__PREDICTION_QUERY_FIELDS__
  var personalCandidates: [InputiaPersonalCandidate] = [], personalPredictions: [InputiaPersonalPrediction] = []
  var personalCode = "ni", personalRefreshGeneration: UInt64 = 0, personalOrderLockedCode: String?
  var personalExpectedSelection: NSRange?, personalPhrase = InputiaPersonalPhraseAssembly()
  var personalEscapeConsumed = false
  var pendingPersonalSelection: PersonalSelection?, personalUndo: PersonalUndo?
  var personalInputDeferral = InputiaPersonalInputDeferral<NSEvent>()
  var personalPredictionPending: Bool { personalInputDeferral.isPending }
  var personalDeferredOrigin: PersonalDeferredOrigin?, personalRecoveryToken: UInt64?
  var personalBoundaryProof: InputiaTargetBridgeReply?
  let personalBoundaryWait = InputiaPersonalBoundaryWait()
  var sharedChineseSelection = Resettable(), sharedEnglishSelection = Resettable(), sharedChineseOrder: Int?
  var sharedEnglishCandidates: [String: String] = [:], explicitEnglishCandidates: [String: String] = [:]
  var hotwordOverlay: (code: String, words: [String], base: [String], snapshot: Int, context: Int)?
  var sharedEnglishRefreshQueued = false, sharedTargetReady = false, sharedPreparedClientIdentity: ObjectIdentifier?
  var latestComposing = "ni", latestCandidates = ["你", "呢"]
  var expandedCandidates: [String] = [], expandedCandidateEntries: [Entry] = [], expandedActiveRowIndex = 0
  var candidatePanelExpanded = false, recallCandidates: [String] = []
  var englishCompletionPrefix = "", englishCompletionCandidates: [String] = [], chineseCandidateRect = NSRect.zero
  let voiceControllerID = "fixture-controller"
  var voiceActivationGeneration: UInt64 = 0, permissionEpoch: UInt64 = 1
  var localCompositionGeneration: UInt64 = 0, localSelectionGeneration: UInt64 = 0
  var typedCompositionOrigin: InputiaVoiceTargetSnapshot.Snapshot?, typedEventOrigin: InputiaVoiceTargetSnapshot.Snapshot?
  var shortcutPreparedSnapshot: InputiaVoiceTargetSnapshot.Snapshot?, voicePreCaptureRetryDeadline: Double?
  var typedRetainedOrigins: [String: Double] = [:], voiceTargetSnapshots: [String: InputiaVoiceTargetSnapshot.Snapshot] = [:]
  var voicePermissionEpochs: [String: UInt64] = [:], attemptedVoiceOutputOperations = Set<String>()
  var memoryViewGeneration: UInt64 = 0, memoryFieldID: String?, memorySelectionIntent: UUID?
  var memoryDisplayedTargets: [String: InputiaVoiceTargetSnapshot.Snapshot] = [:], memoryDisplayedComposing: [String: String] = [:]
  var cachedAppContext: Bool?, pushedAppContext: Bool?
  var captureRequests: [(InputiaVoiceTarget?) -> Void] = []
  var capturePurposes: [Bool] = [], targetCapturePending = false
  init() {
__CALLBACKS__
  }
  func client() -> Client? { live }
  // 模拟原生目标证明入口；调度和回复续接由提取的生产方法负责。
  func prepareUnifiedVoiceTarget(client: Client?, forPrediction: Bool = false,
    completion: @escaping (InputiaVoiceTarget?) -> Void) {
    __CAPTURE_ANCHOR_START__
    capturePurposes.append(forPrediction)
    captureRequests.append { [weak self] target in
      guard let self else { completion(nil); return }
      __CAPTURE_ANCHOR_FINISH__
      completion(target)
    }
  }
  func reportShortcutReadiness(_ reason: String) {}
  func retireWordSpan(reason: String) {}
  func retireMemoryCommit() {}
  func clearClipboardRecall() {}
  func setMarkedComposition(_ text: String, client: Client) { client.marked = NSRange(location: 0, length: text.utf16.count) }
  func clearMarkedText(_ client: Client) { client.marked = emptyReplacementRange }
  func clearHotwordOverlay() { hotwordOverlay = nil }
  func clearSharedChineseCandidates() { sharedChineseOrder = nil }
  @discardableResult func refreshHotwordPrefix(client: Client) -> Bool { false }
  func scheduleSharedEnglishRefresh(client: Client) {}
  func scheduleManagedRank(client: Client) {}
  func prepareMemoryCommit(candidates: [String], prefix: String, client: Client) {}
  func takeMemoryConfirmation(text: String, replacement: NSRange, client: Client) -> (() -> Void)? { nil }
  func discardTypedCompositionOrigin() { typedCompositionOrigin = nil; personalization.reset() }
  func removeRetiredVoiceTarget(_ id: String) { voiceTargetSnapshots.removeValue(forKey: id) }
  func appContext(for client: Client) -> Bool { false }
  func resetShiftInputModeSession(reason: String) {}
  // 原生键入边界：推荐/边界方法使用生产源码，普通未消费文本交真实replay fallback。
  func handle(_ input: NSEvent, client: Client) -> Bool {
    if physicalInput(input) { return true }
    observePersonalKey(input, client: client)
    return handlePersonalCandidateKey(input, client: client) ?? false
  }
__METHODS__
}
func pump(_ interval: Double = 0.01) { RunLoop.main.run(until: Date().addingTimeInterval(interval)) }
func policy() -> InputiaPersonalReply { .init(server_instance: "service", enabled: true, epoch: 1, result: nil, code: nil) }
func event(_ code: UInt16 = 49, text: String = " ", modifiers: NSEvent.ModifierFlags = [],
  timestamp: TimeInterval = ProcessInfo.processInfo.systemUptime, repeated: Bool = false) -> NSEvent {
  NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: timestamp,
    windowNumber: 0, context: nil, characters: text, charactersIgnoringModifiers: text, isARepeat: repeated, keyCode: code)!
}
let controller = Controller()
InputiaHost.activeInputController = controller
controller.personalization.start()
precondition(InputiaVoiceInputLauncher.pending.first?.command.kind == "policy")
InputiaVoiceInputLauncher.pending.removeFirst().completion(policy())
let trigger = controller.predictionTriggerForSpace(event(), client: controller.live)!
let completed = InputiaBridgeOutcome(composing: "", candidates: [], commit: "你好", candidateIDs: [])
controller.bridge.latestOutcome = completed
_ = controller.apply(completed, client: controller.live, predictionTrigger: trigger)
pump()
precondition(controller.live.inserts == 1, "只能由真实Space提交一次")
precondition(InputiaHost.candidatePanel!.hides == 0, "Space到pending不能隐藏原窗")
let originalToken = controller.predictionPresentation.currentToken!
let originalDeadline = controller.predictionPresentation.deadline!
controller.installExpiryCallback()
if __CLEANUP_ONLY__ {
  precondition(controller.bridge.clearManagedMemory())
  controller.bridge.memoryDidExpire?()
  precondition(controller.predictionPresentation.currentToken == originalToken
    && controller.predictionPresentation.deadline == originalDeadline && InputiaHost.candidatePanel!.visible,
    "旧rank到期只能清旧结果，不能撤销独立五秒shell")
  print("predictionLifecycleCleanupSelfCheck=true unchangedDeadline=true")
  controller.personalization.stop()
  exit(0)
}
precondition(controller.captureRequests.count == 1, "已提交文字缺少target时必须异步采集并续接真实query")
precondition(controller.targetCapturePending && controller.capturePurposes == [false],
  "提交前已有普通预捕获时不能并发抢占broker")
controller.captureRequests.removeFirst()(nil)
pump()
precondition(!controller.targetCapturePending && controller.captureRequests.count == 1
  && controller.capturePurposes == [false, true], "旧预捕获失败也必须续接独立fresh推荐capture")
func completeCapture(_ suffix: String) -> InputiaVoiceTarget {
  precondition(controller.captureRequests.count == 1, "每轮只允许一个fresh capture")
  let target = InputiaVoiceTarget(target_id: suffix, activation_generation: controller.voiceActivationGeneration)
  controller.voiceTargetSnapshots[target.target_id] = .init(target)
  controller.captureRequests.removeFirst()(target)
  pump()
  return target
}
func takeQuery() -> InputiaVoiceInputLauncher.Pending {
  let index = InputiaVoiceInputLauncher.pending.firstIndex { $0.command.kind == "query" }
  precondition(index != nil, "必须实际调用真实Personalization.query发送transport请求")
  return InputiaVoiceInputLauncher.pending.remove(at: index!)
}
func response(_ request: InputiaVoiceInputLauncher.Pending, text: String) -> InputiaPersonalReply {
  .init(server_instance: "service", enabled: true, epoch: 1,
    result: .init(ordered_ids: [], predictions: [.init(id: "prediction:fixture", text: text)],
      context_id: request.command.target!.target_id), code: nil)
}
func unchangedShell() {
  precondition(controller.predictionPresentation.currentToken == originalToken
    && controller.predictionPresentation.deadline == originalDeadline && InputiaHost.candidatePanel!.visible,
    "清理和续查询不能延长或撤销原展示deadline")
}
_ = completeCapture("post-commit")
let requests = InputiaVoiceInputLauncher.sent.filter { $0.kind == "query" }
precondition(requests.count == 1 && requests[0].context == "你好" && requests[0].input_code == "",
  "真实Personalization.query必须实际发送本次提交文本，不能被bind清空")
precondition(controller.personalization.context.text.isEmpty, "独立queryContext不能补写学习上下文")
let first = takeQuery()
first.completion(.init(server_instance: "service", enabled: true, epoch: 1, result: nil, code: "target_query_busy"))
pump(0.08)
let retry = takeQuery()
precondition(retry.deadline == first.deadline && retry.command.context == "你好", "busy重试不得延租或丢本次文字")
retry.completion(response(retry, text: "合成推荐一"))
precondition(controller.personalPredictions.map(\.text) == ["合成推荐一"]
  && InputiaHost.candidatePanel!.shown == ["合成推荐一"] && InputiaHost.candidatePanel!.hides == 0,
  "必须经过真实异步回调显示推荐，不能只看pending还在")
unchangedShell()
// 模拟提交前已安装rank的2秒到期通知；CAPI是边界，通知与清理方法是生产代码。
precondition(controller.bridge.clearManagedMemory())
controller.bridge.memoryDidExpire?()
pump()
unchangedShell()
precondition(controller.personalPredictions.isEmpty && controller.personalization.view == nil,
  "不能为保留shell而保留失效推荐或admit view")
_ = completeCapture("after-rank-expiry")
let revokedQuery = takeQuery()
precondition(revokedQuery.command.context == "你好", "缓存到期后fresh query必须仍携本次上屏文字")
InputiaPermissionLifecycle.shared.snapshot.status = .unknown
InputiaPermissionLifecycle.shared.epoch += 1
controller.synchronizePermissionEpoch()
pump()
unchangedShell()
revokedQuery.completion(response(revokedQuery, text: "已撤销的旧结果"))
precondition(controller.personalPredictions.isEmpty && InputiaHost.candidatePanel!.shown.isEmpty,
  "旧target/query的迟到成功不能恢复已撤销内容")
InputiaPermissionLifecycle.shared.snapshot.status = .ready
InputiaPermissionLifecycle.shared.epoch += 1
controller.synchronizePermissionEpoch()
pump()
unchangedShell()
_ = completeCapture("after-permission-recovery")
let freshQuery = takeQuery()
let sentBeforeJoin = InputiaVoiceInputLauncher.sent.filter { $0.kind == "query" }.count
if __CHECK_QUERY_JOIN__ {
  controller.schedulePersonalization(client: controller.live)
  controller.schedulePersonalization(client: controller.live)
  pump()
  precondition(InputiaVoiceInputLauncher.sent.filter { $0.kind == "query" }.count == sentBeforeJoin,
    "相同当前target/context的重复调度不应派发第二条wire")
}
freshQuery.completion(response(freshQuery, text: "合成新鲜推荐"))
precondition(InputiaHost.candidatePanel!.shown == ["合成新鲜推荐"],
  "当前同scope重复query不能自增ticket后丢弃首条真实回复，必须立即交付")
pump()
precondition(InputiaVoiceInputLauncher.sent.filter { $0.kind == "query" }.count == sentBeforeJoin,
  "原回复已经交付后也不能再派发冗余query wire")
unchangedShell()
// 未结束展示前再发一条新查询，随后真实Esc关闭；迟到结果不能重开。
#if PREDICTION_QUERY_CONTEXT
controller.invalidatePredictionTarget()
#else
preconditionFailure("修前代码缺少独立推荐target撤销")
#endif
controller.schedulePersonalization(client: controller.live)
_ = completeCapture("before-escape")
let afterEscape = takeQuery()
precondition(controller.physicalInput(event(53)), "pending期间Esc必须由生产按键边界消费")
afterEscape.completion(response(afterEscape, text: "Esc后的迟到结果"))
precondition(controller.predictionPresentation.currentToken == nil && !InputiaHost.candidatePanel!.visible
  && controller.personalPredictions.isEmpty, "Esc关闭后不允许旧结果复活")
let queryCountAfterEscape = InputiaVoiceInputLauncher.sent.filter { $0.kind == "query" }.count
controller.schedulePersonalization(client: controller.live)
pump()
precondition(InputiaVoiceInputLauncher.sent.filter { $0.kind == "query" }.count == queryCountAfterEscape,
  "策略刷新不能重开已关闭的展示")
precondition(controller.live.inserts == 1 && controller.capturePurposes.dropFirst().allSatisfy { $0 }, "fresh捕获仅只读推荐，不重放输入")
precondition(InputiaVoiceInputLauncher.sent.allSatisfy { $0.kind == "policy" || $0.kind == "query" }, "不得补写学习feedback")
precondition(InputiaTypedCapture.shared.writes == 0, "新推荐上下文不能补写history")
print("predictionLifecycleIntegrationSelfCheck=true realQuery=true currentCommitContext=true freshAfterExpiry=true staleReplyRejected=true unchangedDeadline=true busyKeepsLease=true zeroFeedback=true zeroHistory=true")
controller.personalization.stop()

// 新场景仍走实际Space->capture->query，区别只在合成宿主/transport的回答时机。
func openedQuery() -> (Controller, InputiaVoiceInputLauncher.Pending) {
  let value = Controller()
  InputiaHost.activeInputController = value
  InputiaHost.candidatePanel = InputiaCandidatePanel()
  InputiaPermissionLifecycle.shared.snapshot.status = .ready
  value.permissionEpoch = InputiaPermissionLifecycle.shared.epoch
  value.personalization.start()
  let policyIndex = InputiaVoiceInputLauncher.pending.firstIndex { $0.command.kind == "policy" }!
  InputiaVoiceInputLauncher.pending.remove(at: policyIndex).completion(policy())
  let cause = value.predictionTriggerForSpace(event(), client: value.live)!
  value.bridge.latestOutcome = completed
  _ = value.apply(completed, client: value.live, predictionTrigger: cause)
  precondition(value.captureRequests.count == 1)
  value.captureRequests.removeFirst()(nil)
  pump()
  precondition(value.captureRequests.count == 1)
  let target = InputiaVoiceTarget(target_id: UUID().uuidString,
    activation_generation: value.voiceActivationGeneration)
  value.voiceTargetSnapshots[target.target_id] = .init(target)
  value.captureRequests.removeFirst()(target)
  pump()
  return (value, takeQuery())
}

let (refreshing, preBarrierQuery) = openedQuery()
let originalRefreshToken = refreshing.predictionPresentation.currentToken
let originalRefreshDeadline = refreshing.predictionPresentation.deadline
let policyBeforeBarrier = InputiaVoiceInputLauncher.sent.filter { $0.kind == "policy" }.count
NotificationCenter.default.post(name: Notification.Name("InputiaPrivacyRevoked"), object: nil, userInfo: ["owned_context_action": "preserve"])
pump(0.01)
precondition(InputiaVoiceInputLauncher.sent.filter { $0.kind == "policy" }.count == policyBeforeBarrier + 1,
  "真实barrier后有效五秒窗口必须立即请求policy，不能等待一秒轮询tick")
for _ in 0..<3 { refreshing.schedulePersonalization(client: refreshing.live) }
pump(0.01)
precondition(InputiaVoiceInputLauncher.sent.filter { $0.kind == "policy" }.count == policyBeforeBarrier + 1,
  "有效窗口连续刷新必须共享一条在途policy")
precondition(refreshing.predictionPresentation.currentToken == originalRefreshToken
  && refreshing.predictionPresentation.deadline == originalRefreshDeadline,
  "主动policy刷新不能重开窗口或延长原五秒deadline")
preBarrierQuery.completion(response(preBarrierQuery, text: "屏障前旧结果"))
precondition(refreshing.personalPredictions.isEmpty, "立即刷新policy也不能沿用屏障前旧query")
let immediatePolicyIndex = InputiaVoiceInputLauncher.pending.firstIndex { $0.command.kind == "policy" }!
InputiaVoiceInputLauncher.pending.remove(at: immediatePolicyIndex).completion(policy())
pump()
precondition(refreshing.captureRequests.count == 1, "新policy成功后须走fresh target证明再续查询")
let refreshedTarget = InputiaVoiceTarget(target_id: UUID().uuidString,
  activation_generation: refreshing.voiceActivationGeneration)
refreshing.voiceTargetSnapshots[refreshedTarget.target_id] = .init(refreshedTarget)
refreshing.captureRequests.removeFirst()(refreshedTarget)
pump()
let refreshedQuery = takeQuery()
refreshedQuery.completion(response(refreshedQuery, text: "立即恢复的推荐"))
precondition(InputiaHost.candidatePanel!.shown == ["立即恢复的推荐"]
  && refreshing.predictionPresentation.deadline == originalRefreshDeadline,
  "立即策略恢复必须实际显示fresh推荐，且沿用原展示期限")
// 新一轮真实barrier若主动policy失败，只能等既有定时轮询，不能回调递归猛发。
let policiesBeforeFailure = InputiaVoiceInputLauncher.sent.filter { $0.kind == "policy" }.count
NotificationCenter.default.post(name: Notification.Name("InputiaPrivacyRevoked"), object: nil, userInfo: ["owned_context_action": "preserve"])
pump(0.01)
let failingPolicyIndex = InputiaVoiceInputLauncher.pending.firstIndex { $0.command.kind == "policy" }!
InputiaVoiceInputLauncher.pending.remove(at: failingPolicyIndex).completion(nil)
for _ in 0..<3 { refreshing.schedulePersonalization(client: refreshing.live) }
pump(0.02)
precondition(InputiaVoiceInputLauncher.sent.filter { $0.kind == "policy" }.count == policiesBeforeFailure + 1
  && !refreshing.personalization.allowed && refreshing.personalPredictions.isEmpty,
  "主动策略失败必须保持fail closed，重复schedule不能产生快速重试环")
refreshing.personalization.stop()
refreshing.closePredictionPresentation()
print("predictionPolicyLatencySelfCheck=true immediateAfterBarrier=true policySingleflight=true freshProofRequired=true originalDeadlinePreserved=true noFailureRetryLoop=true")

let (moved, oldFieldQuery) = openedQuery()
moved.live = Client() // 同controller、相同坐标的新字段也不能继承本次文字。
moved.schedulePersonalization(client: moved.live)
oldFieldQuery.completion(response(oldFieldQuery, text: "错误字段的迟到结果"))
precondition(moved.predictionPresentation.currentToken == nil && moved.personalPredictions.isEmpty
  && !InputiaHost.candidatePanel!.visible, "queryContext不能跨client/字段")
moved.personalization.stop()

let (disabled, disabledQuery) = openedQuery()
disabledQuery.completion(.init(server_instance: "service", enabled: false, epoch: 1, result: nil, code: nil))
precondition(disabled.predictionPresentation.currentToken == nil
  && !InputiaHost.candidatePanel!.visible, "明确disabled必须由真实policyChanged关闭shell")
disabled.personalization.stop()

let (expired, lateQuery) = openedQuery()
let cutoff = expired.predictionPresentation.deadline!
// 运行真正的五秒主队列计时器；无NSApplication、窗口、真实输入或用户profile。
while ProcessInfo.processInfo.systemUptime < cutoff + 0.03 {
  pump(min(0.05, max(0.001, cutoff + 0.03 - ProcessInfo.processInfo.systemUptime)))
  // 只应答真实policy轮询，不延长展示期限；query仍延迟到关闭之后。
  while let index = InputiaVoiceInputLauncher.pending.firstIndex(where: { $0.command.kind == "policy" }) {
    InputiaVoiceInputLauncher.pending.remove(at: index).completion(policy())
  }
}
precondition(expired.predictionPresentation.currentToken == nil && !InputiaHost.candidatePanel!.visible,
  "真实五秒timer必须关闭，不能只在轮询时被动过期")
lateQuery.completion(response(lateQuery, text: "五秒后的迟到结果"))
precondition(expired.personalPredictions.isEmpty && !InputiaHost.candidatePanel!.visible,
  "过期后policy刷新和迟到结果都不能复活")
precondition(expired.live.inserts == 1 && InputiaTypedCapture.shared.writes == 0)
precondition(InputiaVoiceInputLauncher.sent.allSatisfy { $0.kind == "policy" || $0.kind == "query" })
expired.personalization.stop()
print("predictionLifecycleBoundaries=true preCaptureSingleflight=true fieldIsolation=true explicitDisabledCloses=true realFiveSecondTimer=true lateReplyCannotReopen=true")

#if PREDICTION_ACCEPT_CONTEXT
func verifyAcceptedKey(_ code: UInt16 = 49, text: String = " ", expectedIndex: Int) {
let (accepted, acceptedQuery) = openedQuery()
let choices = [InputiaPersonalPrediction(id: "prediction:first", text: "后续第一词"),
  InputiaPersonalPrediction(id: "prediction:second", text: "后续第二词")]
acceptedQuery.completion(.init(server_instance: "service", enabled: true, epoch: 1,
  result: .init(ordered_ids: [], predictions: choices, context_id: acceptedQuery.command.target!.target_id), code: nil))
let acceptedTarget = acceptedQuery.command.target!
let beforeAcceptanceWrites = InputiaTypedCapture.shared.writes
let feedbackCountBefore = InputiaVoiceInputLauncher.sent.filter { $0.kind == "feedback" }.count
let hidesBefore = InputiaHost.candidatePanel!.hides
let deadlineBefore = accepted.predictionPresentation.deadline
let input = event(code, text: text)
precondition(!accepted.physicalInput(input), "选择键不能被前置predictionBoundary当作取消消费")
accepted.observePersonalKey(input, client: accepted.live)
precondition(accepted.personalPredictions == choices && InputiaHost.candidatePanel!.hides == hidesBefore
  && accepted.predictionPresentation.deadline == deadlineBefore,
  "真实boundary和observe不能提前清掉Space/数字键要选择的推荐或重置deadline")
#if PREDICTION_KEYS
precondition(accepted.handlePersonalCandidateKey(input, client: accepted.live) == true,
  "必须由真实按键入口发起推荐准入，不能直接调用accept来伪证按键可用")
#else
preconditionFailure("缺少普通候选语义的推荐按键入口")
#endif
precondition(!InputiaVoiceInputLauncher.released.contains(acceptedTarget.target_id),
  "关闭五秒展示不能释放已转交给实际选择的fresh target")
let admissionIndex = InputiaVoiceInputLauncher.pending.firstIndex { $0.command.kind == "admit" }!
let admission = InputiaVoiceInputLauncher.pending.remove(at: admissionIndex)
precondition(admission.command.context == "你好" && admission.command.target == acceptedTarget
  && admission.command.prediction_id == choices[expectedIndex].id && admission.command.text == choices[expectedIndex].text,
  "实际按键必须选中正确text/id，准入仍核对原短上下文和fresh target")
admission.completion(.init(server_instance: "service", enabled: true, epoch: 1,
  result: .init(ordered_ids: nil, predictions: nil, context_id: acceptedTarget.target_id,
    admitted: true, prediction_id: choices[expectedIndex].id), code: nil))
precondition(accepted.live.inserts == 1 && InputiaVoiceInputLauncher.targetReplies.count == 1,
  "仅admit成功不能跳过最终原字段证明直接插入")
InputiaVoiceInputLauncher.targetReplies.removeFirst()(.init(target: acceptedTarget))
let feedback = Array(InputiaVoiceInputLauncher.sent.filter { $0.kind == "feedback" }.dropFirst(feedbackCountBefore))
precondition(accepted.live.inserts == 2 && feedback.count == 1 && feedback[0].previous == "你好"
  && feedback[0].text == choices[expectedIndex].text && feedback[0].target == acceptedTarget
  && feedback[0].original_rank == expectedIndex,
  "真实成功接受推荐后只反馈一次，previous必须沿用已准入View的queryContext")
precondition(InputiaTypedCapture.shared.writes == beforeAcceptanceWrites + 1,
  "仅记录本次真实接受的输入，不补录初始Space或查询")
let feedbackIndex = InputiaVoiceInputLauncher.pending.firstIndex { $0.command.kind == "feedback" }!
InputiaVoiceInputLauncher.pending.remove(at: feedbackIndex).completion(
  .init(server_instance: "service", enabled: true, epoch: 1, result: nil, code: "target_query_busy"))
pump(0.12)
precondition(InputiaVoiceInputLauncher.sent.filter { $0.kind == "feedback" }.count == feedbackCountBefore + 1
  && accepted.live.inserts == 2, "反馈忙碌不能重发写入或重放上屏")
accepted.personalization.stop()
}
verifyAcceptedKey(expectedIndex: 0)
verifyAcceptedKey(18, text: "1", expectedIndex: 0)
verifyAcceptedKey(19, text: "2", expectedIndex: 1)
verifyAcceptedKey(48, text: "\t", expectedIndex: 0)

#if PREDICTION_KEYS
for input in [event(25, text: "9"), event(modifiers: [.control, .shift]), event(36, text: "\r")] {
  let (passthrough, request) = openedQuery()
  request.completion(response(request, text: "不能误选的词"))
  let admissionsBefore = InputiaVoiceInputLauncher.sent.filter { $0.kind == "admit" }.count
  precondition(!passthrough.physicalInput(input))
  passthrough.observePersonalKey(input, client: passthrough.live)
  precondition(passthrough.handlePersonalCandidateKey(input, client: passthrough.live) == nil
    && !passthrough.personalPredictionPending && passthrough.live.inserts == 1
    && InputiaVoiceInputLauncher.sent.filter { $0.kind == "admit" }.count == admissionsBefore,
    "超范围数字/语音快捷键/Return必须交还后续原输入流程，不能误选推荐")
  passthrough.personalization.stop()
}
let composing = Controller()
InputiaHost.activeInputController = composing
InputiaHost.candidatePanel = InputiaCandidatePanel()
for input in [event(), event(18, text: "1"), event(19, text: "2"), event(36, text: "\r")] {
  precondition(!composing.physicalInput(input))
  composing.observePersonalKey(input, client: composing.live)
  precondition(composing.handlePersonalCandidateKey(input, client: composing.live) == nil
    && composing.latestComposing == "ni" && composing.latestCandidates == ["你", "呢"]
    && InputiaHost.candidatePanel!.hides == 0,
    "非空普通拼音继续交原候选路径，不能被推荐按键入口截走或清窗")
}
#endif
print("predictionAcceptanceSelfCheck=true actualSpaceOneTwoTab=true actualBoundaryAndObserver=true correctTextIDRank=true modifierAndRangePassthrough=true composingRoutePreserved=true transferredTargetRetained=true oneFeedbackPerAcceptance=true previousContextPreserved=true noFeedbackReplay=true")

#if PREDICTION_KEYS
func takeCommand(_ kind: String) -> InputiaVoiceInputLauncher.Pending {
  let index = InputiaVoiceInputLauncher.pending.firstIndex { $0.command.kind == kind }
  precondition(index != nil, "真实业务链必须发出预期transport命令")
  return InputiaVoiceInputLauncher.pending.remove(at: index!)
}
func finishFreshCapture(_ value: Controller) -> InputiaVoiceTarget {
  precondition(value.captureRequests.count == 1)
  let target = InputiaVoiceTarget(target_id: UUID().uuidString, activation_generation: value.voiceActivationGeneration)
  value.voiceTargetSnapshots[target.target_id] = .init(target)
  value.captureRequests.removeFirst()(target)
  pump()
  return target
}
func admitSelected(_ request: InputiaVoiceInputLauncher.Pending) {
  request.completion(.init(server_instance: "service", enabled: true, epoch: 1,
    result: .init(ordered_ids: nil, predictions: nil, context_id: request.command.target!.target_id,
      admitted: true, prediction_id: request.command.prediction_id), code: nil))
}
let (chain, initialChainQuery) = openedQuery()
var chainQuery = initialChainQuery, expectedContext = "你好"
let chainFeedbackBefore = InputiaVoiceInputLauncher.sent.filter { $0.kind == "feedback" }.count
let chainHidesBefore = InputiaHost.candidatePanel!.hides
var replayOrigin: Controller.PersonalDeferredOrigin?
var replayProof: InputiaTargetBridgeReply?
for round in 0..<3 {
  let choices = [InputiaPersonalPrediction(id: "chain-\(round)-one", text: "续词甲\(round)"),
    InputiaPersonalPrediction(id: "chain-\(round)-two", text: "续词乙\(round)")]
  chainQuery.completion(.init(server_instance: "service", enabled: true, epoch: 1,
    result: .init(ordered_ids: [], predictions: choices, context_id: chainQuery.command.target!.target_id), code: nil))
  let tokenBefore = chain.predictionPresentation.currentToken!
  let deadlineBefore = chain.predictionPresentation.deadline!
  let selectedIndex = round == 1 ? 1 : 0
  let input = selectedIndex == 1 ? event(19, text: "2") : event()
  precondition(chain.handle(input, client: chain.live), "每轮真实按键入口都应开始选择")
  precondition(chain.predictionSelectionHandoff != nil && chain.predictionPresentation.currentToken == nil
    && InputiaHost.candidatePanel!.visible && InputiaHost.candidatePanel!.shown.isEmpty
    && InputiaHost.candidatePanel!.hides == chainHidesBefore,
    "选择等待期应原窗空白，不沿用旧候选也不隐藏")
  let admission = takeCommand("admit")
  precondition(admission.command.context == expectedContext && admission.command.text == choices[selectedIndex].text)
  replayOrigin = chain.personalDeferredOrigin
  admitSelected(admission)
  precondition(chain.live.inserts == round + 1, "最终字段核验完成前不得插入")
  let proof = InputiaTargetBridgeReply(target: admission.command.target)
  replayProof = proof
  InputiaVoiceInputLauncher.targetReplies.removeFirst()(proof)
  expectedContext = String((expectedContext + choices[selectedIndex].text).suffix(64))
  precondition(chain.live.inserts == round + 2 && chain.predictionSelectionHandoff == nil
    && chain.predictionPresentation.currentToken != nil && chain.predictionPresentation.currentToken != tokenBefore
    && chain.predictionPresentation.deadline! > deadlineBefore
    && chain.predictionQueryContext?.text == choices[selectedIndex].text,
    "只有真实成功插入才启动新五秒；捕获前只暂存最新片段")
  let feedback = takeCommand("feedback")
  precondition(feedback.command.text == choices[selectedIndex].text && feedback.command.original_rank == selectedIndex
    && feedback.command.previous == admission.command.context)
  feedback.completion(policy())
  _ = finishFreshCapture(chain)
  chainQuery = takeQuery()
  precondition(chainQuery.command.context == expectedContext && InputiaHost.candidatePanel!.hides == chainHidesBefore,
    "连续联想必须实际发送新的累积context且全程无hide闪烁")
}
precondition(InputiaVoiceInputLauncher.sent.filter { $0.kind == "feedback" }.count == chainFeedbackBefore + 3)
chainQuery.completion(response(chainQuery, text: "不得自动连选"))
let presentedAt = chain.predictionDisplayedAt!
precondition(chain.predictionSelectionIndex(event(timestamp: presentedAt - 0.01)) == nil
  && chain.predictionSelectionIndex(event(repeated: true)) == nil,
  "早于新列表显示的系统排队键与键盘连发不能选中后来的推荐")
let wouldSelect = event()
precondition(chain.predictionSelectionIndex(wouldSelect) == 0)
let admissionsBeforeReplay = InputiaVoiceInputLauncher.sent.filter { $0.kind == "admit" }.count
let insertsBeforeReplay = chain.live.inserts
chain.replayDeferredPersonalInput([wouldSelect, event(18, text: "1")], pending: replayOrigin!, proof: replayProof!)
precondition(chain.live.inserts == insertsBeforeReplay + 2
  && InputiaVoiceInputLauncher.sent.filter { $0.kind == "admit" }.count == admissionsBeforeReplay
  && chain.personalInputReplayDepth == 0,
  "真实replay应保留已消费的普通空格/文字，但绝不能把它们变成新推荐的选择")
chain.personalization.stop()
chain.closePredictionPresentation()

for cancelWithEscape in [true, false] {
  let (cancelled, request) = openedQuery()
  request.completion(response(request, text: "不可迟到插入"))
  precondition(cancelled.handle(event(), client: cancelled.live))
  let admission = takeCommand("admit")
  let beforeFeedback = InputiaVoiceInputLauncher.sent.filter { $0.kind == "feedback" }.count
  if cancelWithEscape {
    precondition(cancelled.physicalInput(event(53)), "等待准入时Esc必须消费并结束本轮")
    admitSelected(admission)
  } else {
    admitSelected(admission)
    InputiaVoiceInputLauncher.targetReplies.removeFirst()(nil)
  }
  precondition(cancelled.live.inserts == 1 && cancelled.predictionPresentation.currentToken == nil
    && cancelled.predictionSelectionHandoff == nil && !InputiaHost.candidatePanel!.visible
    && InputiaVoiceInputLauncher.sent.filter { $0.kind == "feedback" }.count == beforeFeedback,
    "Esc或最终proof缺失必须停止等待，迟到admit不能插入/续联想/补写feedback")
  cancelled.personalization.stop()
}
print("predictionContinuationSelfCheck=true threeActualSelections=true cumulativeQueryContext=true newDeadlineOnlyAfterInsert=true continuousBlankHandoff=true staleAndRepeatKeysRejected=true actualReplayCannotAutoSelect=true escapedOrUnprovenHandoffCannotInsert=true")

// 普通拼音仍走真实 apply/insert；仅底层 Rime 产出的候选与捕获 transport 是合成边界。
func commitNextPinyin(_ value: Controller, text: String,
  field: InputiaConfirmedFieldContext.Field?, boundary: NSEvent? = nil,
  startOverride: Int? = nil, deriveFieldFromRegistry: Bool = false) -> InputiaVoiceInputLauncher.Pending {
  if let boundary { _ = value.physicalInput(boundary) }
  _ = value.physicalInput(event(0, text: "a"))
  let start = startOverride ?? value.live.selection.location
  value.live.selection = NSRange(location: start, length: 0)
  value.live.marked = NSRange(location: start, length: 2)
  value.latestComposing = "ni"
  value.bridge.latestOutcome = InputiaBridgeOutcome()
  let beforeMarked = value.confirmedFieldContext.generation
  value.observeConfirmedFieldInput(event(45, text: "n"), client: value.live)
  if startOverride == nil && boundary == nil {
    precondition(value.confirmedFieldContext.generation == beforeMarked,
      "当前正文尾部的marked组合不能追加或清空已确认正文")
  }
  let cause = value.predictionTriggerForSpace(event(), client: value.live)!
  let next = InputiaBridgeOutcome(composing: "", candidates: [], commit: text, candidateIDs: [])
  value.bridge.latestOutcome = next
  _ = value.apply(next, client: value.live, predictionTrigger: cause)
  // 普通history预捕获可能先占位；失败只恢复fresh预测捕获，不补写旧history。
  if value.targetCapturePending {
    value.captureRequests.removeFirst()(nil); pump()
  }
  precondition(value.captureRequests.count == 1)
  let target = InputiaVoiceTarget(target_id: UUID().uuidString, activation_generation: value.voiceActivationGeneration)
  let snapshot = InputiaVoiceTargetSnapshot.Snapshot(target)
  if deriveFieldFromRegistry {
    let now = ProcessInfo.processInfo.systemUptime
    InputiaVoiceInputLauncher.nativeLeases = InputiaVoiceInputLauncher.nativeLeases.filter { $0.value.expiresAt > now }
    let inherited = InputiaVoiceInputLauncher.nativeLeases.values.first?.field
      ?? .init(server: "service", instance: UUID().uuidString)
    snapshot.confirmedField = inherited
    InputiaVoiceInputLauncher.nativeLeases[target.target_id] = (inherited, snapshot.expiresAt)
  } else { snapshot.confirmedField = field }
  value.voiceTargetSnapshots[target.target_id] = snapshot
  value.captureRequests.removeFirst()(target); pump()
  return takeQuery()
}
for change in ["same", "field", "server", "missing", "gap", "escape", "arrow"] {
  let (typing, initial) = openedQuery()
  initial.completion(response(initial, text: "不会自动选择"))
  let same = InputiaConfirmedFieldContext.Field(server: "service", instance: "field-a")
  let proof: InputiaConfirmedFieldContext.Field? = change == "missing" ? nil
    : .init(server: change == "server" ? "new-service" : same.server,
      instance: change == "field" ? "new-field" : same.instance)
  let boundary = change == "escape" ? event(53) : (change == "arrow" ? event(123) : nil)
  let beforeFeedback = InputiaVoiceInputLauncher.sent.filter { $0.kind == "feedback" }.count
  let query = commitNextPinyin(typing, text: "世界", field: proof, boundary: boundary,
    startOverride: change == "gap" ? 15 : nil)
  precondition(query.command.context == (change == "same" ? "你好世界" : "世界"),
    "仅同既有field+server证明且caret连续才能拼接；同proxy同位置、服务换代、缺证明和编辑均隔离")
  precondition(typing.live.inserts == 2
    && InputiaVoiceInputLauncher.sent.filter { $0.kind == "feedback" }.count == beforeFeedback,
    "上下文累积仅供只读query，不能添加学习写入或重复插入")
  query.completion(response(query, text: "隔离结果"))
  typing.personalization.stop(); typing.closePredictionPresentation()
}
// 长只读context进入真实Query与Admit；学习Feedback仍限定最后64个scalar。
let (longTyping, longInitial) = openedQuery()
longInitial.completion(response(longInitial, text: "旧推荐"))
let longQuery = commitNextPinyin(longTyping, text: String(repeating: "文", count: 400),
  field: .init(server: "service", instance: "field-a"))
precondition(longQuery.command.context?.unicodeScalars.count == 402)
longQuery.completion(response(longQuery, text: "下一词"))
precondition(longTyping.handle(event(), client: longTyping.live))
let longAdmission = takeCommand("admit")
precondition(longAdmission.command.context == longQuery.command.context)
admitSelected(longAdmission)
InputiaVoiceInputLauncher.targetReplies.removeFirst()(.init(target: longAdmission.command.target))
let shortFeedback = takeCommand("feedback")
precondition(shortFeedback.command.previous?.unicodeScalars.count == 64,
  "长查询上下文不能扩大既有Feedback.previous合同")
shortFeedback.completion(policy())
longTyping.personalization.stop(); longTyping.closePredictionPresentation()
print("predictionConfirmedFieldSelfCheck=true ordinaryPinyinAccumulates=true changedFieldAndServerIsolated=true gapEscapeArrowReset=true markedIsNotCommit=true queryAdmit512Feedback64=true")

// selectedRange 是可重入原生边界；旧栈帧不能擦除内层成功登记的新片段。
for path in ["observe", "record", "current"] {
  let (reentrant, prior) = openedQuery()
  prior.completion(response(prior, text: "旧推荐"))
  var innerSegment: UInt64?, innerToken: UInt64?
  reentrant.live.selectionHook = {
    reentrant.live.selection = NSRange(location: 21, length: 0)
    reentrant.recordConfirmedFieldCommit("新", start: 20, client: reentrant.live)
    innerSegment = reentrant.confirmedFieldContext.pendingToken
    if path == "current" {
      reentrant.predictionInputGeneration += 1
      reentrant.beginPredictionAfterCommit(.init(generation: reentrant.predictionInputGeneration,
        client: ObjectIdentifier(reentrant.live)), text: "新", start: 20, client: reentrant.live)
      innerToken = reentrant.predictionPresentation.currentToken
    }
  }
  if path == "observe" { reentrant.observeConfirmedFieldInput(event(0, text: "a"), client: reentrant.live) }
  else if path == "record" { reentrant.recordConfirmedFieldCommit("旧", start: 0, client: reentrant.live) }
  else { precondition(reentrant.currentPredictionToken(client: reentrant.live) == nil) }
  precondition(innerSegment != nil && reentrant.confirmedFieldContext.pendingToken == innerSegment,
    "旧IMK查询返回后不得reset内层新commit")
  if path == "current" {
    precondition(innerToken != nil && reentrant.predictionPresentation.currentToken == innerToken,
      "旧currentToken查询不得关闭内层新窗口")
  }
  reentrant.personalization.stop(); reentrant.closePredictionPresentation()
}
let emptyObserver = Controller()
InputiaHost.activeInputController = emptyObserver
let readsBeforeEmpty = emptyObserver.live.selectionReads
emptyObserver.observeConfirmedFieldInput(event(0, text: "a"), client: emptyObserver.live)
precondition(emptyObserver.live.selectionReads == readsBeforeEmpty,
  "无字段上下文时不能为每个普通按键增加IMK选区查询")
print("predictionContextReentrancySelfCheck=true staleObservePreservesNewCommit=true staleRecordPreservesNewCommit=true staleCurrentPreservesNewWindow=true emptyContextNoExtraQuery=true")

// 字段实例按仍存活的真实ownership分配；没有旧租约就生成不同字段实例，不能固定返回field-a。
for boundary in ["capture", "escape", "expiry", "failure"] {
  let (anchored, query) = openedQuery()
  query.completion(response(query, text: "不选择，继续拼音"))
  let original = anchored.predictionTarget!
  if boundary == "expiry" { original.expiresAt = ProcessInfo.processInfo.systemUptime + 0.03 }
  let originalExpiry = original.expiresAt
  InputiaVoiceInputLauncher.nativeLeases = [original.targetID: (original.confirmedField!, original.expiresAt)]
  let releasesBefore = InputiaVoiceInputLauncher.released.filter { $0 == original.targetID }.count
  _ = anchored.physicalInput(event(0, text: "a"))
  precondition(anchored.confirmedFieldAnchor === original && anchored.predictionTarget == nil
    && anchored.typedCompositionOrigin == nil && original.expiresAt == originalExpiry
    && InputiaVoiceInputLauncher.nativeLeases[original.targetID] != nil,
    "普通继续输入只转交仍有效的只读anchor，不进入typed/admit或延长expiry")
  _ = anchored.physicalInput(event(45, text: "n"))
  precondition(anchored.confirmedFieldAnchor === original, "后续普通按键不能提前释放交接租约")
  if boundary == "capture" {
    let next = commitNextPinyin(anchored, text: "世界", field: nil, deriveFieldFromRegistry: true)
    precondition(next.command.context == "你好世界"
      && anchored.predictionTarget?.confirmedField == original.confirmedField,
      "新capture必须在旧lease释放之前继承同字段，真实Query才可累积前文")
    next.completion(response(next, text: "新字段证明的结果"))
  } else if boundary == "escape" { _ = anchored.physicalInput(event(53)) }
  else if boundary == "expiry" { pump(0.05) }
  else {
    anchored.prepareUnifiedVoiceTarget(client: anchored.live, forPrediction: true) { _ in }
    anchored.captureRequests.removeFirst()(nil)
  }
  precondition(anchored.confirmedFieldAnchor == nil
    && InputiaVoiceInputLauncher.nativeLeases[original.targetID] == nil
    && InputiaVoiceInputLauncher.released.filter { $0 == original.targetID }.count == releasesBefore + 1,
    "成功交接、失败、Esc、原到期均必须且只能release旧anchor一次")
  anchored.releaseConfirmedFieldAnchor(matching: original.targetID)
  precondition(InputiaVoiceInputLauncher.released.filter { $0 == original.targetID }.count == releasesBefore + 1)
  anchored.personalization.stop(); anchored.closePredictionPresentation()
}
print("predictionAnchorOwnershipSelfCheck=true fieldDerivedFromLiveLeases=true captureBeforeRelease=true noExpiryExtension=true neverAdmitOrigin=true releaseOnceOnSuccessFailureEscapeExpiry=true")

for action in ["clear_prefix", "clear_all", "unknown"] {
  InputiaVoiceInputLauncher.pending.removeAll()
  let (privacy, initial) = openedQuery()
  initial.completion(response(initial, text: "旧推荐"))
  let request = commitNextPinyin(privacy, text: "世界", field: .init(server: "service", instance: "field-a"))
  precondition(request.command.context == "你好世界")
  let token = privacy.predictionPresentation.currentToken, deadline = privacy.predictionPresentation.deadline
  let info: [AnyHashable: Any]? = action == "unknown" ? nil : ["owned_context_action": action]
  NotificationCenter.default.post(name: Notification.Name("InputiaPrivacyRevoked"), object: nil, userInfo: info)
  precondition(!privacy.confirmedFieldContext.hasContext)
  request.completion(response(request, text: "撤销前迟到结果"))
  precondition(privacy.personalPredictions.allSatisfy { $0.text != "撤销前迟到结果" })
  if action == "clear_prefix" {
    precondition(privacy.predictionPresentation.currentToken == token
      && privacy.predictionPresentation.deadline == deadline && privacy.predictionQueryContext?.text == "世界")
    let renewedPolicy = takeCommand("policy")
    renewedPolicy.completion(policy()); pump()
    _ = finishFreshCapture(privacy)
    let renewedQuery = takeQuery()
    precondition(renewedQuery.command.context == "世界", "首次握手只可继续本次片段，不能恢复旧前缀")
    renewedQuery.completion(response(renewedQuery, text: "新许可结果"))
  } else {
    precondition(privacy.predictionPresentation.currentToken == nil && privacy.predictionQueryContext == nil,
      "真实撤销和未知通知必须清本轮文本与token")
  }
  privacy.personalization.stop(); privacy.closePredictionPresentation()
}
// 没有屏障通知但有效policy身份/learning epoch变化，也不能保留自有上下文。
for serverChanges in [false, true] {
  InputiaVoiceInputLauncher.pending.removeAll()
  let model = InputiaPersonalization()
  var actions: [String] = []
  model.ownContextInvalidated = { actions.append($0) }
  model.start(); takeCommand("policy").completion(policy())
  precondition(actions.isEmpty, "第一次正常policy不能误作真实隐私撤销")
  pump(1.05)
  takeCommand("policy").completion(.init(server_instance: serverChanges ? "other" : "service",
    enabled: true, epoch: serverChanges ? 1 : 2, result: nil, code: nil))
  precondition(actions == ["clear_all"], "已知服务或learning epoch变化必须撤销自有正文")
  model.stop()
}
print("predictionOwnedPrivacySelfCheck=true handshakeDropsPrefixOnly=true realAndUnknownRevocationDropsText=true knownPolicyIdentityChangeClears=true staleReplyCannotRestore=true")
#endif
#endif

// 跨scope保留最新等待请求，但不能把新上下文或新字段并入旧wire的授权。
for changesTarget in [false, true] {
  let model = InputiaPersonalization()
  model.start()
  let policyIndex = InputiaVoiceInputLauncher.pending.firstIndex { $0.command.kind == "policy" }!
  InputiaVoiceInputLauncher.pending.remove(at: policyIndex).completion(policy())
  let oldTarget = InputiaVoiceTarget(target_id: UUID().uuidString)
  let newTarget = changesTarget ? InputiaVoiceTarget(target_id: UUID().uuidString) : oldTarget
  var oldDeliveries = 0, newDeliveries = 0
  model.query(target: oldTarget, code: "", schemaID: "luna", candidates: [],
    predictionContext: "原上下文", isCurrent: { true }) { _ in oldDeliveries += 1 }
  pump()
  let oldRequest = takeQuery()
  model.query(target: newTarget, code: "", schemaID: "luna", candidates: [],
    predictionContext: changesTarget ? "原上下文" : "新上下文", isCurrent: { true }) { _ in newDeliveries += 1 }
  pump()
  oldRequest.completion(response(oldRequest, text: "旧scope回复"))
  pump()
  precondition(oldDeliveries == 0 && newDeliveries == 0, "不同scope不能借旧回复交付给新callback")
  let newRequest = takeQuery()
  precondition(newRequest.command.target == newTarget
    && newRequest.command.context == (changesTarget ? "原上下文" : "新上下文"),
    "不同target或context必须发送自己的wire请求")
  newRequest.completion(response(newRequest, text: "新scope回复"))
  precondition(oldDeliveries == 0 && newDeliveries == 1, "只有新scope自己的当前回复可以交付")
  model.stop()
}
print("predictionQueryLatencySelfCheck=true sameScopeDeliveredWithoutSecondWire=true differentContextNotMerged=true differentTargetNotMerged=true")
'''

with tempfile.TemporaryDirectory(prefix="inputia-prediction-lifecycle-") as temporary:
    directory = Path(temporary)
    swift = directory / "main.swift"
    query_fields = """
  var predictionQueryContext: PredictionQueryContext?
  var predictionTarget: InputiaVoiceTargetSnapshot.Snapshot?
  var predictionTargetCapture: UUID?
  var predictionQueryGeneration: UInt64 = 0, predictionActivationGeneration: UInt64 = 0
""" if "  private struct PredictionQueryContext" in source else ""
    swift.write_text(fixture.replace("__METHODS__", methods)
        .replace("__PREDICTION_QUERY_FIELDS__", query_fields)
        .replace("__CALLBACKS__", callbacks)
        .replace("__CAPTURE_ANCHOR_START__", capture_anchor_start)
        .replace("__CAPTURE_ANCHOR_FINISH__", capture_anchor_finish)
        .replace("__CHECK_QUERY_JOIN__", "false" if args.scenario == "latency-policy" else "true")
        .replace("__CLEANUP_ONLY__", "true" if args.scenario == "cleanup" else "false"))
    binary = directory / "check"
    flags = ["-D", "PREDICTION_QUERY_CONTEXT"] if query_fields else []
    if has_accept_context:
        flags += ["-D", "PREDICTION_ACCEPT_CONTEXT"]
    if has_prediction_keys:
        flags += ["-D", "PREDICTION_KEYS"]
    subprocess.run([
        "/usr/bin/swiftc", "-D", "INPUTIA_PAIRED_BUILD", *flags,
        str(root / "Sources/InputiaInputMethod/InputiaPredictionPresentation.swift"),
        str(root / "Sources/InputiaInputMethod/InputiaConfirmedFieldContext.swift"),
        str(root / "Sources/InputiaInputMethod/InputiaShortcutClassifier.swift"),
        str(args.personalization_source or root / "Sources/InputiaInputMethod/InputiaPersonalization.swift"),
        str(swift), "-o", str(binary),
    ], check=True)
    raise SystemExit(subprocess.run([str(binary)], check=False).returncode)
