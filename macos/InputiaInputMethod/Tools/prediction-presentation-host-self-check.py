#!/usr/bin/env python3
"""执行真实 Space 提交/显示接线，使用无 GUI 的 IMK 客户端替身。"""
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
source = (root / "Sources/InputiaInputMethod/main.swift").read_text()

def extract(marker, source=source):
    start = source.index(marker)
    if source[:start].endswith("  @discardableResult\n"):
        start -= len("  @discardableResult\n")
    brace = source.index("{", start)
    depth, end = 1, brace + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end].replace("private ", "")

methods = "\n".join(extract(marker) for marker in [
    "  private struct PredictionCommitTrigger",
    "  private struct PredictionQueryContext",
    "  private func invalidatePredictionTarget(",
    "  private var ownedCandidatePanel:",
    "  private func predictionTriggerForSpace(",
    "  private func predictionSelectionIndex(",
    "  private func closePredictionPresentation(",
    "  private func retirePredictionPresentation()",
    "  private func currentPredictionToken(",
    "  private func beginPredictionAfterCommit(",
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
    "  private func updateCandidateWindow(",
])
panel_source = (root / "Sources/InputiaInputMethod/InputiaCandidatePanel.swift").read_text()
restoration_method = extract("  func isRestorationCurrent(", source=panel_source)
# 直接执行 handle 中的生产按键边界，验证 Esc 发生在异步结果之前也会关闭。
input_boundary = extract("    if event.type == .keyDown || (event.type == .flagsChanged && changesInput)")
methods += "\nfunc physicalInput(_ event: NSEvent) -> Bool {\nvar predictionTransitionKeptWindow = false\nlet changesInput = true\n" + input_boundary + "\nreturn false\n}\n"

# 执行生产异步回复体，保持查询次序并注入 IMK 验证期间关闭会话。
query_callback = extract("    self.personalization.query(target: target, code: code, schemaID: schemaID, candidates: pool,")
callback_body = query_callback.split("{ [weak self] view in", 1)[1].rsplit("}", 1)[0]
methods += """
func deliverPredictionReply(_ view: View, token predictionToken: UInt64?, client: Client) {
  let version = personalRefreshGeneration, code = latestComposing, page = bridge.latestOutcome.page
  let candidateIDs = bridge.latestOutcome.candidateIDs, candidateTexts = bridge.latestOutcome.candidates
  let schemaID = bridge.schemaID, selection = client.selectedRange(), target = 1
  let predictionIsCurrent = { self.currentPredictionToken(client: client) == predictionToken }
  let callback: (View) -> Void = { [weak self] view in
""" + callback_body + "\n}\ncallback(view)\n}\n"

# 确认 Space 原因贯通所有已存在的异步选择入口，不能靠 explicit=false 推测。
for marker in ["  private func choosePersonal(", "  private func chooseNativeWithLearning(",
               "  private func selectHotwordOverlay(", "  private func enqueueSharedChineseSelection(",
               "  private func commitExpandedCandidate(columnIndex:",
               "  private func commitExpandedCandidate(_ entry:",
               "  private func acceptAdmittedPersonalCandidate("]:
    assert "predictionTrigger: predictionTrigger" in extract(marker), marker
query = extract("  private func schedulePersonalization(")
assert "guard !code.isEmpty || predictionToken != nil" in query
assert "self.currentPredictionToken(client: client) == predictionToken" in query
assert "showPredictionEmpty" in query

fixture = r'''
import AppKit
let keyCodeSpace: UInt16 = 49, keyCodeTab: UInt16 = 48, keyCodeEscape: UInt16 = 53
let emptyReplacementRange = NSRange(location: NSNotFound, length: 0)
func inputiaDebugLog(_ value: String) {}
struct InputiaBridgeOutcome {
  var ok = true, consumed = true, mode = "Chinese", composing = "ni", candidates = ["你", "呢"]
  var commit: String? = nil
  var candidateIDs: [String] = ["a", "b"]
  var page = 0
}
final class Client {
  var selection = NSRange(location: 2, length: 0), marked = NSRange(location: 0, length: 2)
  var inserts = 0, acceptsInsert = true
  var duringSelection: (() -> Void)?
  func selectedRange() -> NSRange { duringSelection?(); return selection }
  func markedRange() -> NSRange { marked }
  func attributes(forCharacterIndex: Int, lineHeightRectangle: inout NSRect) {}
  func insertText(_ text: String, replacementRange: NSRange) {
    inserts += 1
    if acceptsInsert {
      let start = replacementRange.location == NSNotFound ? selection.location : replacementRange.location
      selection = NSRange(location: start + text.utf16.count, length: 0)
      marked = emptyReplacementRange
    }
  }
}
typealias IMKTextInput = Client
final class Bridge { var latestOutcome = InputiaBridgeOutcome(); var schemaID = "test" }
struct Prediction: Equatable { var text: String }
struct Candidate { var text: String }
struct View { var predictions: [Prediction] = []; var candidates: [Candidate] = [] }
final class Origin {
  let inputiaTarget = 1
  let targetID = "prediction-target"
  var expiresAt = ProcessInfo.processInfo.systemUptime + 120
  var reusableForShortcut: Bool { ProcessInfo.processInfo.systemUptime < expiresAt }
  var duringValidation: (() -> Void)?
  func isCurrentForTypedOrigin(client: Client, controllerID: String, activationGeneration: UInt64) -> Bool {
    duringValidation?(); return true
  }
}
enum InputiaPersonalContext {
  static func matchesFirstPage(page: Int, expectedPage: Int, ids: [String], expectedIDs: [String]) -> Bool {
    page == expectedPage && ids == expectedIDs
  }
}
final class InputiaCandidatePanel {
  var hides = 0, pending = 0, shows = 0, emptyShows = 0, visible = true, shown = ["你", "呢"]
  var restorationToken: UInt64?
  __RESTORATION_METHOD__
  func hide() { restorationToken = nil; hides += 1; visible = false; shown = [] }
  func showPredictionPending(near: NSRect) { restorationToken = nil; pending += 1; visible = true; shown = [] }
  func showPredictionEmpty(near: NSRect) { restorationToken = nil; emptyShows += 1; visible = true; shown = [] }
  func show(candidates: [String], near: NSRect, expanded: Bool = false,
            primaryCandidateCount: Int = 0, activeRowIndex: Int = 0) {
    restorationToken = nil; shows += 1; visible = true; shown = candidates
  }
  var highlighted = 0
  func showPredictions(_ predictions: [String], highlighted: Int, near: NSRect) {
    restorationToken = nil; shows += 1; visible = true; shown = predictions; self.highlighted = highlighted
  }
}
enum InputiaHost {
  static var activeInputController: Controller?
  static var candidatePanel: InputiaCandidatePanel? = InputiaCandidatePanel()
}
final class InputiaTypedCapture {
  static let shared = InputiaTypedCapture()
  func resetSegment() {}
}
enum InputiaHostTextPolicy {
  static func commitReplacementRange(previousComposing: String, markedRange: NSRange) -> NSRange { markedRange }
  static func shouldContinueMarkedTextAfterCommit(committedText: String?, nextComposing: String) -> Bool { !nextComposing.isEmpty }
  static func shouldClearMarkedText(previousComposing: String, nextComposing: String, committedText: String?) -> Bool { !previousComposing.isEmpty && nextComposing.isEmpty }
  static func candidatesForPanel(composing: String, candidates: [String]) -> [String] { candidates }
}
final class InputiaPermissionLifecycle {
  enum Status { case ready, maintenance }
  struct Snapshot { var status = Status.ready }
  static let shared = InputiaPermissionLifecycle()
  var snapshot = Snapshot()
}
var secure = false
func IsSecureEventInputEnabled() -> Bool { secure }
func isCurrentInputiaSourceSelected() -> Bool { InputiaHost.activeInputController?.sourceSelected == true }
struct Entry { var candidateID: String? = "old" }
struct Resettable { mutating func cancel() {} }
enum InputiaVoiceInputLauncher { static func releaseTarget(_ id: String) {} }
final class Personalization {
  var explicitlyDisabled = false
  var changed: (() -> Void)?
  func invalidateView() { changed?() }
}
final class Controller {
  var confirmedFieldAnchor: Origin?
  var confirmedFieldAnchorExpiry: DispatchWorkItem?
  var confirmedFieldContext = InputiaConfirmedFieldContext()
  var predictionPresentation = InputiaPredictionPresentation()
  var predictionInputGeneration: UInt64 = 1
  var predictionQueryContext: PredictionQueryContext?
  var predictionTarget: Origin?
  var predictionTargetCapture: UUID?
  var predictionQueryGeneration: UInt64 = 0, predictionActivationGeneration: UInt64 = 0
  var predictionExpiry: DispatchWorkItem?
  var predictionClient: ObjectIdentifier?, predictionSelection: NSRange?
  var candidateDisplayCleanupDepth = 0, predictionHandoffDepth = 0
  var predictionSelectionHandoff: UInt64?, predictionDisplayedAt: TimeInterval?
  var predictionHighlight = 0, predictionAnchorRect = NSRect.zero
  var personalInputReplayDepth = 0
  var finishedDeferralTokens: [UInt64] = []
  func finishDeferredPersonalInput(_ token: UInt64) { finishedDeferralTokens.append(token) }
  var personalPredictions = [Prediction(text: "旧推荐")], personalCandidates = [Candidate(text: "你")], personalCode = "ni"
  var personalRefreshGeneration: UInt64 = 0
  var personalOrderLockedCode: String?
  var sharedChineseOrder: Int? = 1, sharedChineseSelection = Resettable()
  var hotwordOverlay: (code: String, words: [String], base: [String], snapshot: Int, context: Int)? = nil
  let personalization = Personalization(), bridge = Bridge()
  let live = Client()
  var sourceSelected = true, latestComposing = "ni", latestCandidates = ["你", "呢"]
  var expandedCandidateEntries = [Entry()], expandedCandidates = ["你"], expandedActiveRowIndex = 0
  var candidatePanelExpanded = false, englishCompletionPrefix = "", englishCompletionCandidates: [String] = []
  var chineseCandidateRect = NSRect.zero
  var queries = 0
  let voiceControllerID = "test"
  var voiceActivationGeneration: UInt64 = 0
  var typedCompositionOrigin: Origin? = Origin()
  var personalExpectedSelection: NSRange?
  func clearHotwordOverlay() { hotwordOverlay = nil }
  init() { personalization.changed = { [weak self] in self?.clearPersonalDisplay() } }
  func client() -> Client? { live }
  func retireWordSpan(reason: String) {}
  func clearClipboardRecall() {}
  func setMarkedComposition(_ text: String, client: Client) { client.marked = NSRange(location: 0, length: text.utf16.count) }
  func clearMarkedText(_ client: Client) { client.marked = emptyReplacementRange }
  @discardableResult func refreshHotwordPrefix(client: Client) -> Bool { false }
  func schedulePersonalization(client: Client) { if currentPredictionToken(client: client) != nil { queries += 1 } }
  func scheduleSharedEnglishRefresh(client: Client) {}
  func scheduleManagedRank(client: Client) {}
  func prepareMemoryCommit(candidates: [String], prefix: String, client: Client) {}
  func takeMemoryConfirmation(text: String, replacement: NSRange, client: Client) -> (() -> Void)? { nil }
  func typedOriginBeforeInsertion(_ client: Client) -> Int? { nil }
  func recordTypedCommit(_ text: String, client: Client, start: Int, origin: Int?) {}
  func recordPersonalCommit(_ text: String, client: Client, start: Int, origin: Int?) {
    personalization.invalidateView()
    schedulePersonalization(client: client)
  }
__METHODS__
}
func event(_ code: UInt16, flags: NSEvent.ModifierFlags = []) -> NSEvent {
  NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: flags, timestamp: ProcessInfo.processInfo.systemUptime,
    windowNumber: 0, context: nil, characters: " ", charactersIgnoringModifiers: " ",
    isARepeat: false, keyCode: code)!
}
func fresh() -> Controller {
  let c = Controller()
  InputiaHost.activeInputController = c
  InputiaHost.candidatePanel = InputiaCandidatePanel()
  secure = false
  InputiaPermissionLifecycle.shared.snapshot.status = .ready
  return c
}
let completed = InputiaBridgeOutcome(composing: "", candidates: [], commit: "你", candidateIDs: [])
var checks = 0
// 真正提交路径触发同步 invalidateView -> changed；仍保持同一 panel，零 hide。
let c = fresh()
let trigger = c.predictionTriggerForSpace(event(49), client: c.live)!
c.bridge.latestOutcome = completed
_ = c.apply(completed, client: c.live, predictionTrigger: trigger)
precondition(c.live.inserts == 1 && c.latestComposing.isEmpty && c.latestCandidates.isEmpty)
precondition(c.personalPredictions.isEmpty && c.personalCandidates.isEmpty)
precondition(InputiaHost.candidatePanel!.hides == 0 && InputiaHost.candidatePanel!.pending == 1)
precondition(c.predictionPresentation.currentToken != nil && c.queries > 0)
let deadline = c.predictionPresentation.deadline!
for _ in 0..<4 { c.schedulePersonalization(client: c.live) }
precondition(c.predictionPresentation.deadline == deadline)
checks += 1
// Esc/关闭后，晚 policy/query 即使再调 schedule 也不会恢复。
precondition(c.physicalInput(event(53)))
let queriesAfterClose = c.queries
c.schedulePersonalization(client: c.live)
precondition(c.queries == queriesAfterClose && !InputiaHost.candidatePanel!.visible)
checks += 1
for key in [UInt16(36), 48, 18, 0] {
  let other = fresh()
  precondition(other.predictionTriggerForSpace(event(key), client: other.live) == nil)
  other.bridge.latestOutcome = completed
  _ = other.apply(completed, client: other.live)
  precondition(other.predictionPresentation.currentToken == nil)
  checks += 1
}
for flags: NSEvent.ModifierFlags in [[.control, .shift], [.shift], [.command], [.option]] {
  let modified = fresh()
  precondition(modified.predictionTriggerForSpace(event(49, flags: flags), client: modified.live) == nil)
  checks += 1
}
// 部分上屏仍有拼音组合，不启动推荐计时。
let partial = fresh()
let partialTrigger = partial.predictionTriggerForSpace(event(49), client: partial.live)!
let part = InputiaBridgeOutcome(composing: "hao", candidates: ["好"], commit: "你", candidateIDs: ["c"])
partial.bridge.latestOutcome = part
_ = partial.apply(part, client: partial.live, predictionTrigger: partialTrigger)
precondition(partial.predictionPresentation.currentToken == nil && partial.latestComposing == "hao")
precondition(InputiaHost.candidatePanel!.shown == ["好"])
checks += 1
// 异步 Space 原因不可跨下一次按键/退出代数复用。
let late = fresh()
let lateTrigger = late.predictionTriggerForSpace(event(49), client: late.live)!
late.retirePredictionPresentation()
late.bridge.latestOutcome = completed
_ = late.apply(completed, client: late.live, predictionTrigger: lateTrigger)
precondition(late.predictionPresentation.currentToken == nil)
checks += 1
let rejected = fresh()
let rejectedTrigger = rejected.predictionTriggerForSpace(event(49), client: rejected.live)!
rejected.live.acceptsInsert = false
rejected.bridge.latestOutcome = completed
_ = rejected.apply(completed, client: rejected.live, predictionTrigger: rejectedTrigger)
precondition(rejected.predictionPresentation.currentToken == nil)
checks += 1
// 开始下一段拼音时仅换内容，旧关闭/计时不能隐藏新候选。
let typing = fresh()
let typingTrigger = typing.predictionTriggerForSpace(event(49), client: typing.live)!
typing.bridge.latestOutcome = completed
_ = typing.apply(completed, client: typing.live, predictionTrigger: typingTrigger)
precondition(!typing.physicalInput(event(0)))
let typed = InputiaBridgeOutcome(composing: "hao", candidates: ["好"], candidateIDs: ["c"])
typing.bridge.latestOutcome = typed
_ = typing.apply(typed, client: typing.live)
typing.closePredictionPresentation()
precondition(InputiaHost.candidatePanel!.hides == 0 && InputiaHost.candidatePanel!.shown == ["好"])
checks += 1
// 查询同步切换 owner 后既不能新建，也不能返回旧展示 token。
let reentrant = fresh(), replacement = Controller()
let reentrantTrigger = reentrant.predictionTriggerForSpace(event(49), client: reentrant.live)!
reentrant.latestComposing = ""; reentrant.bridge.latestOutcome = completed
reentrant.live.selection = NSRange(location: 1, length: 0)
reentrant.live.duringSelection = { InputiaHost.activeInputController = replacement }
reentrant.beginPredictionAfterCommit(reentrantTrigger, text: "你", start: 0, client: reentrant.live)
precondition(reentrant.predictionPresentation.currentToken == nil && InputiaHost.candidatePanel!.pending == 0)
checks += 1
InputiaHost.activeInputController = reentrant
reentrant.live.duringSelection = nil
reentrant.beginPredictionAfterCommit(reentrantTrigger, text: "你", start: 0, client: reentrant.live)
reentrant.live.duringSelection = { InputiaHost.activeInputController = replacement }
precondition(reentrant.currentPredictionToken(client: reentrant.live) == nil)
checks += 1
// 维护时既不能新建，也立即关闭已存在的推荐。
let maintenance = fresh()
let maintenanceTrigger = maintenance.predictionTriggerForSpace(event(49), client: maintenance.live)!
maintenance.latestComposing = ""; maintenance.bridge.latestOutcome = completed
maintenance.live.selection = NSRange(location: 1, length: 0)
InputiaPermissionLifecycle.shared.snapshot.status = .maintenance
maintenance.beginPredictionAfterCommit(maintenanceTrigger, text: "你", start: 0, client: maintenance.live)
precondition(maintenance.predictionPresentation.currentToken == nil)
checks += 1
InputiaPermissionLifecycle.shared.snapshot.status = .ready
maintenance.beginPredictionAfterCommit(maintenanceTrigger, text: "你", start: 0, client: maintenance.live)
InputiaPermissionLifecycle.shared.snapshot.status = .maintenance
precondition(maintenance.currentPredictionToken(client: maintenance.live) == nil && !InputiaHost.candidatePanel!.visible)
checks += 1
// 真实异步回复在 typed origin 的 IMK 验证中遇到 Esc，不得重显空推荐窗。
let lateEmpty = fresh()
let emptyTrigger = lateEmpty.predictionTriggerForSpace(event(49), client: lateEmpty.live)!
lateEmpty.bridge.latestOutcome = completed
_ = lateEmpty.apply(completed, client: lateEmpty.live, predictionTrigger: emptyTrigger)
let emptyToken = lateEmpty.predictionPresentation.currentToken!
lateEmpty.predictionTarget = lateEmpty.typedCompositionOrigin
lateEmpty.typedCompositionOrigin?.duringValidation = { _ = lateEmpty.physicalInput(event(53)) }
lateEmpty.deliverPredictionReply(View(), token: emptyToken, client: lateEmpty.live)
precondition(InputiaHost.candidatePanel!.emptyShows == 0 && !InputiaHost.candidatePanel!.visible)
precondition(lateEmpty.predictionPresentation.currentToken == nil)
checks += 1

// 上屏时不能提前信任旧前缀；只有新的字段证明才能合并它。
let continuation = fresh()
continuation.latestComposing = ""
continuation.bridge.latestOutcome = completed
let continued = Controller.PredictionCommitTrigger(generation: continuation.predictionInputGeneration,
  client: ObjectIdentifier(continuation.live), contextPrefix: "你好", continuesSelection: true)
continuation.predictionSelectionHandoff = 7
continuation.insertCommittedText("世界", client: continuation.live,
  replacementRange: NSRange(location: 0, length: 0), predictionTrigger: continued)
precondition(continuation.predictionQueryContext?.text == "世界")
precondition(continuation.predictionQueryContext?.confirmedSegment != nil)
precondition(continuation.predictionSelectionHandoff == nil && InputiaHost.candidatePanel!.hides == 0)
checks += 1
// 空白交接期间 Esc 同样消费并清理，不能留下没有计时器的窗口。
let handoff = fresh()
handoff.latestComposing = ""
handoff.personalPredictions = []
handoff.predictionSelectionHandoff = 8
precondition(handoff.physicalInput(event(53)))
precondition(handoff.predictionSelectionHandoff == nil && !InputiaHost.candidatePanel!.visible)
precondition(handoff.finishedDeferralTokens == [8])
checks += 1
// 只有列表已经显示之后的新物理空格才能选当前高亮项；队列重放和自动重复均不准入。
let keys = fresh()
keys.latestComposing = ""
keys.personalPredictions = [Prediction(text: "甲"), Prediction(text: "乙")]
let displayed = ProcessInfo.processInfo.systemUptime
keys.predictionDisplayedAt = displayed
keys.predictionHighlight = 1
func selectionEvent(timestamp: TimeInterval, repeated: Bool = false, text: String = " ",
  keyCode: UInt16 = 49) -> NSEvent {
  NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: [], timestamp: timestamp,
    windowNumber: 0, context: nil, characters: text, charactersIgnoringModifiers: text,
    isARepeat: repeated, keyCode: keyCode)!
}
precondition(keys.predictionSelectionIndex(selectionEvent(timestamp: displayed + 0.01)) == 1)
precondition(keys.predictionSelectionIndex(selectionEvent(timestamp: displayed - 0.01)) == nil)
precondition(keys.predictionSelectionIndex(selectionEvent(timestamp: displayed + 0.01, repeated: true)) == nil)
keys.personalInputReplayDepth = 1
precondition(keys.predictionSelectionIndex(selectionEvent(timestamp: displayed + 0.01)) == nil)
keys.personalInputReplayDepth = 0
// 数字键与 Tab 不再选择推荐：数字照常输入，Tab 只移动高亮。
precondition(keys.predictionSelectionIndex(selectionEvent(timestamp: displayed + 0.01, text: "2", keyCode: 19)) == nil)
precondition(keys.predictionSelectionIndex(selectionEvent(timestamp: displayed + 0.01, text: "\t", keyCode: 48)) == nil)
checks += 6

print("predictionPresentationHostSelfCheck=true checks=\(checks)")
'''
with tempfile.TemporaryDirectory(prefix="inputia-prediction-host-") as directory:
    swift = Path(directory) / "main.swift"
    binary = Path(directory) / "check"
    swift.write_text(fixture.replace("__METHODS__", methods).replace("__RESTORATION_METHOD__", restoration_method))
    subprocess.run(["swiftc", "-D", "INPUTIA_PAIRED_BUILD", str(root / "Sources/InputiaInputMethod/InputiaPredictionPresentation.swift"),
                    str(root / "Sources/InputiaInputMethod/InputiaConfirmedFieldContext.swift"),
                    str(swift), "-o", str(binary)], check=True)
    subprocess.run([str(binary)], check=True)
