#!/usr/bin/env python3
"""以替身宿主执行真实清理方法，不启动 App 或访问用户 profile。"""

from pathlib import Path
import argparse
import subprocess
import tempfile


root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--main-source", type=Path)
args = parser.parse_args()
source = (args.main_source or root / "Sources/InputiaInputMethod/main.swift").read_text()
def extract(marker):
    start = source.index(marker)
    brace = source.index("{", start)
    depth = 1
    end = brace + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end].replace("private ", "")

methods = "\n".join(extract(marker) for marker in [
    "  private var ownedCandidatePanel:",
    "  private func clearPersonalDisplay()",
    "  func synchronizePermissionEpoch()",
    "  override func hidePalettes()",
    "  private func updateCandidateWindow(client:",
])
# 方法签名中包含默认闭包，末尾锚点避免把它误认为函数体。
start = source.index("  @discardableResult\n  private func clearManagedMemoryDisplay(")
end = source.index("  private func observePersonalKey(", start)
methods += "\n" + source[start:end].replace("private ", "")

fixture = r'''
import AppKit
struct InputiaBridgeOutcome {
  var ok = true, mode = "Chinese", composing = "ni", candidates = ["你", "呢"], candidateIDs: [String] = []
}
final class Client {
  var selection = NSRange(location: 2, length: 0)
  var marked = NSRange(location: 0, length: 2)
  var duringAttributes: (() -> Void)?
  func attributes(forCharacterIndex: Int, lineHeightRectangle: inout NSRect) { duringAttributes?() }
  func selectedRange() -> NSRange { selection }
  func markedRange() -> NSRange { marked }
}
typealias IMKTextInput = Client
enum InputiaHostTextPolicy {
  static func candidatesForPanel(composing: String, candidates: [String]) -> [String] { candidates }
}
final class Bridge {
  var latestOutcome = InputiaBridgeOutcome(candidates: ["旧私密候选"])
  var clearedOutcome = InputiaBridgeOutcome()
  var clearSucceeds = true, clears = 0
  var duringClear: (() -> Void)?
  func clearManagedMemory() -> Bool {
    clears += 1
    duringClear?()
    if clearSucceeds { latestOutcome = clearedOutcome }
    return clearSucceeds
  }
  func cancelManagedMemoryRequests() {}
}
final class InputiaCandidatePanel {
  var visible = true, shown: [String] = ["旧私密候选"]
  var writes = 0, sequence: UInt64 = 0
  var restoration: UInt64?
  var events: [String] = []
  func clearForRestoration() -> UInt64 {
    sequence &+= 1; restoration = sequence; writes += 1; shown = []; events.append("blank")
    return sequence
  }
  func isRestorationCurrent(_ token: UInt64) -> Bool { restoration == token }
  func hide() { restoration = nil; writes += 1; visible = false; shown = []; events.append("hide") }
  func showPredictionPending(near: NSRect) { restoration = nil; writes += 1; shown = []; visible = true; events.append("pending") }
  func show(candidates: [String], near: NSRect, expanded: Bool,
            primaryCandidateCount: Int, activeRowIndex: Int) {
    restoration = nil; writes += 1; shown = candidates; visible = true; events.append("show")
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
  var isReady: Bool {
    get { snapshot.status == .ready }
    set { snapshot.status = newValue ? .ready : .unknown }
  }
}
var secure = false
func IsSecureEventInputEnabled() -> Bool { secure }
func isCurrentInputiaSourceSelected() -> Bool { InputiaHost.activeInputController?.sourceSelected == true }
enum InputiaSecureDirectPolicy {
  static func shouldUseSecureDirectMode(context: Bool) -> Bool { context }
}
final class InputiaSharedTermsMemory {
  static let shared = InputiaSharedTermsMemory()
  var duringClear: (() -> Void)?
  func clear() { duringClear?() }
}
struct Target { let targetID = "old-target" }
enum InputiaVoiceInputLauncher { static func releaseTarget(_ id: String) {} }
struct Pending { let token = 1 }
final class CaptureScope { func invalidate() {} }
struct Resettable {
  mutating func reset() {}
  mutating func cancel() {}
  mutating func finish(_ token: Int) -> Int? { token }
}
final class Personalization {
  var changed: (() -> Void)?
  var duringReset: (() -> Void)?
  func reset() { changed?(); duringReset?() }
}
struct Entry { var candidateID: String? = "old-id" }
class BaseController {
  var paletteCalls = 0
  func hidePalettes() { paletteCalls += 1 }
}
final class Controller: BaseController {
  let targetCaptureScope = CaptureScope()
  var candidateDisplayCleanupDepth = 0, predictionHandoffDepth = 0
  var predictionDisplayedAt: TimeInterval?
  func retirePredictionPresentation() {}
  func invalidatePredictionTarget(cancelCapture: Bool = true) {}
  func schedulePersonalization(client: Client) {}
  func currentPredictionToken(client: Client) -> UInt64? { nil }
  var confirmedFieldContext = InputiaConfirmedFieldContext()
  func clearConfirmedFieldContext() { confirmedFieldContext.reset() }
  func releaseConfirmedFieldAnchor() {}
  var permissionEpoch: UInt64 = .max, voiceActivationGeneration: UInt64 = 0
  var voiceTargetSnapshots = ["old": Target()], voicePermissionEpochs = ["old": UInt64(0)]
  var attemptedVoiceOutputOperations: Set<String> = ["old"]
  var shortcutPreparedSnapshot: Target? = Target(), voicePreCaptureRetryDeadline: Int? = 1
  var sharedTargetReady = true, sharedPreparedClientIdentity: Int? = 1
  var sensitiveContext = false
  var duringAppContext: (() -> Void)?
  var chineseCandidateRect = NSRect.zero
  override init() {
    super.init()
    personalization.changed = { [weak self] in self?.clearPersonalDisplay() }
  }
  func discardTypedCompositionOrigin() {}
  func clearSharedChineseCandidates() {}
  func appContext(for client: Client) -> Bool { duringAppContext?(); return sensitiveContext }
  let bridge = Bridge()
  var live: Client? = Client(), sourceSelected = true
  var memoryViewGeneration: UInt64 = 1, personalRefreshGeneration: UInt64 = 1
  var memoryFieldID: String? = "old-field", memorySelectionIntent: Int? = 1
  var memoryDisplayedTargets = ["rank": Target()], memoryDisplayedComposing = ["rank": "ni"]
  var personalDeferredOrigin: Pending? = Pending()
  var personalInputDeferral = Resettable(), personalPhrase = Resettable()
  let personalization = Personalization()
  var personalRecoveryToken: Int? = 1, personalBoundaryProof: Int? = 1
  var pendingPersonalSelection: Int? = 1, personalUndo: Int? = 1
  var personalCandidates = ["秘密"], personalPredictions = ["秘密"], personalCode = "ni"
  var personalOrderLockedCode: String? = "ni"
  var sharedEnglishSelection = Resettable(), sharedChineseSelection = Resettable()
  var sharedChineseOrder: Int? = 1
  var hotwordOverlay: (code: String, words: [String], base: [String], snapshot: Int, context: Int)? = ("ni", ["secret"], ["你"], 1, 1)
  var sharedEnglishCandidates = ["secret": 1], explicitEnglishCandidates = ["secret": 1]
  var sharedEnglishRefreshQueued = true
  var latestComposing = "ni", latestCandidates = ["秘密"], expandedCandidates = ["秘密"]
  var expandedCandidateEntries = [Entry()], expandedActiveRowIndex = 1
  var recallCandidates = ["秘密"], englishCompletionCandidates = ["secret"], englishCompletionPrefix = "sec"
  var candidatePanelExpanded = true, cachedAppContext: Int? = 1, pushedAppContext: Int? = 1
  func client() -> Client? { live }
  func retireWordSpan(reason: String) {}
  func retireMemoryCommit() {}
  func removeRetiredVoiceTarget(_ id: String) {}
__ACTUAL_METHODS__
}
var checks = 0
func scenario(_ name: String, visible: Bool, restoring: Bool = true,
              configure: (Controller) -> Void = { _ in }) {
  let controller = Controller()
  InputiaHost.activeInputController = controller
  InputiaHost.candidatePanel = InputiaCandidatePanel()
  InputiaPermissionLifecycle.shared.isReady = true
  InputiaSharedTermsMemory.shared.duringClear = nil
  secure = false
  configure(controller)
  controller.clearManagedMemoryDisplay(restoringBasicCandidates: restoring)
  precondition(controller.memoryDisplayedTargets.isEmpty, name)
  precondition(controller.memoryDisplayedComposing.isEmpty, name)
  precondition(controller.personalCandidates.isEmpty && controller.personalPredictions.isEmpty, name)
  precondition(controller.sharedEnglishCandidates.isEmpty && controller.explicitEnglishCandidates.isEmpty, name)
  precondition(controller.recallCandidates.isEmpty && controller.englishCompletionCandidates.isEmpty, name)
  precondition(controller.hotwordOverlay == nil && controller.sharedChineseOrder == nil, name)
  precondition(InputiaHost.candidatePanel!.visible == visible, name)
  precondition(controller.latestCandidates == (visible ? ["你", "呢"] : []), name)
  if visible { precondition(InputiaHost.candidatePanel!.shown == ["你", "呢"], name) }
  precondition(controller.bridge.clears == (restoring ? 1 : 0), name)
  checks += 1
}
scenario("学习域不可用后仅恢复本次清理所得基础候选", visible: true)
scenario("CAPI失败不得复用旧ok结果", visible: false) { $0.bridge.clearSucceeds = false }
scenario("secure input", visible: false) { _ in secure = true }
scenario("增强权限未知仍显示基础候选", visible: true) { _ in InputiaPermissionLifecycle.shared.isReady = false }
scenario("维护拒绝恢复", visible: false) { _ in InputiaPermissionLifecycle.shared.snapshot.status = .maintenance }
scenario("敏感应用拒绝恢复", visible: false) { $0.sensitiveContext = true }
scenario("已切换输入源", visible: false) { $0.sourceSelected = false }
scenario("字段已失去", visible: false) { $0.live = nil }
scenario("字段在清理时切换", visible: false) { controller in
  controller.bridge.duringClear = { controller.live = Client() }
}
scenario("光标在清理时移动", visible: false) { controller in
  controller.bridge.duringClear = { controller.live?.selection.location = 9 }
}
scenario("组合范围在清理时改变", visible: false) { controller in
  controller.bridge.duringClear = { controller.live?.marked.length = 1 }
}
scenario("清理结果失败", visible: false) { $0.bridge.clearedOutcome.ok = false }
scenario("英文不恢复旧补全", visible: false) { $0.bridge.clearedOutcome.mode = "English" }
scenario("无组合不恢复", visible: false) { $0.bridge.clearedOutcome.composing = "" }
scenario("无基础候选不恢复", visible: false) { $0.bridge.clearedOutcome.candidates = [] }
scenario("默认退出路径继续隐藏", visible: false, restoring: false)
scenario("组合在清理时改变", visible: false) { controller in
  controller.personalization.duringReset = { controller.bridge.latestOutcome.composing = "hao" }
}
scenario("reset同步回调不提前渲染", visible: true) { controller in
  controller.personalization.duringReset = {
    precondition(InputiaHost.candidatePanel!.events == ["blank"] && InputiaHost.candidatePanel!.shown.isEmpty)
    precondition(controller.personalCandidates.isEmpty && controller.personalPredictions.isEmpty)
  }
}
// 非活动控制器不能重画另一个控制器的候选窗。
let inactive = Controller()
InputiaHost.activeInputController = Controller()
InputiaHost.candidatePanel = InputiaCandidatePanel()
inactive.latestComposing = ""
inactive.clearManagedMemoryDisplay(restoringBasicCandidates: true)
precondition(inactive.latestCandidates.isEmpty && InputiaHost.candidatePanel!.shown == ["旧私密候选"])
inactive.hidePalettes()
precondition(InputiaHost.candidatePanel!.writes == 0 && inactive.paletteCalls == 1)
checks += 1

// ready/unknown/ready 都撤销增强映射，一次 epoch 只清一次，并在全部同步回调后恢复。
let epochController = Controller()
InputiaHost.activeInputController = epochController
InputiaHost.candidatePanel = InputiaCandidatePanel()
secure = false
InputiaSharedTermsMemory.shared.duringClear = {
  precondition(epochController.candidateDisplayCleanupDepth > 0)
  precondition(epochController.latestCandidates.isEmpty)
  epochController.clearPersonalDisplay()
}
for (index, state) in [InputiaPermissionLifecycle.Status.ready, .unknown, .ready].enumerated() {
  InputiaPermissionLifecycle.shared.snapshot.status = state
  InputiaPermissionLifecycle.shared.epoch = UInt64(index + 10)
  epochController.synchronizePermissionEpoch()
  epochController.synchronizePermissionEpoch()
  precondition(epochController.bridge.clears == index + 1)
  precondition(InputiaHost.candidatePanel!.shown == ["你", "呢"])
  precondition(epochController.voiceTargetSnapshots.isEmpty && epochController.voicePermissionEpochs.isEmpty)
  precondition(epochController.attemptedVoiceOutputOperations.isEmpty && !epochController.sharedTargetReady)
  checks += 1
}
InputiaSharedTermsMemory.shared.duringClear = nil

// 到期通知使用紧邻 CAPI 清理的新快照，不能再次 clear；失败快照不复用旧 ok。
let expired = Controller()
InputiaHost.activeInputController = expired
InputiaHost.candidatePanel = InputiaCandidatePanel()
precondition(expired.bridge.clearManagedMemory())
expired.managedMemoryWasCleared()?()
precondition(expired.bridge.clears == 1 && InputiaHost.candidatePanel!.shown == ["你", "呢"])
expired.bridge.latestOutcome.ok = false
expired.managedMemoryWasCleared()?()
precondition(expired.bridge.clears == 1 && !InputiaHost.candidatePanel!.visible)
checks += 2

// 全局屏障先清全部 controller，再统一恢复；后续失去 owner/字段不得使用待恢复闭包。
let active = Controller(), previous = Controller()
InputiaHost.activeInputController = active
InputiaHost.candidatePanel = InputiaCandidatePanel()
precondition(active.bridge.clearManagedMemory() && previous.bridge.clearManagedMemory())
let restoreActive = active.managedMemoryWasCleared()
let writesBeforeInactive = InputiaHost.candidatePanel!.writes
let restorePrevious = previous.managedMemoryWasCleared()
precondition(InputiaHost.candidatePanel!.writes == writesBeforeInactive)
precondition(InputiaHost.candidatePanel!.visible && InputiaHost.candidatePanel!.shown.isEmpty)
restoreActive?(); restorePrevious?()
precondition(InputiaHost.candidatePanel!.shown == ["你", "呢"])
precondition(active.bridge.clears == 1 && previous.bridge.clears == 1)
checks += 1
let staleRestore = active.managedMemoryWasCleared()
InputiaHost.activeInputController = previous
InputiaHost.candidatePanel = InputiaCandidatePanel()
staleRestore?()
precondition(InputiaHost.candidatePanel!.writes == 0)
checks += 1
let fieldRestore = previous.managedMemoryWasCleared()
previous.live?.selection.location = 100
fieldRestore?()
precondition(!InputiaHost.candidatePanel!.visible)
checks += 1
// IMK 同步查询中切换 controller，也不能使用查询前捕获的共享 panel。
let querying = Controller(), newOwner = Controller()
InputiaHost.activeInputController = querying
InputiaHost.candidatePanel = InputiaCandidatePanel()
querying.live?.duringAttributes = { InputiaHost.activeInputController = newOwner }
querying.updateCandidateWindow(client: querying.live!)
precondition(InputiaHost.candidatePanel!.writes == 0)
checks += 1
InputiaHost.activeInputController = querying
querying.live?.duringAttributes = nil
querying.duringAppContext = { InputiaHost.activeInputController = newOwner }
querying.clearManagedMemoryDisplay(restoringBasicCandidates: true)
precondition(querying.latestCandidates.isEmpty && !InputiaHost.candidatePanel!.visible)
checks += 1

// 同一基础候选刷新没有隐藏/重开，旧正文在任何宿主检查前已从窗口清除。
let stable = Controller()
InputiaHost.activeInputController = stable
InputiaHost.candidatePanel = InputiaCandidatePanel()
secure = false; InputiaPermissionLifecycle.shared.isReady = true
stable.bridge.latestOutcome = InputiaBridgeOutcome()
stable.bridge.clearedOutcome = stable.bridge.latestOutcome
stable.latestCandidates = ["你", "呢"]
stable.personalCandidates = []; stable.personalPredictions = []
stable.expandedCandidateEntries = []; stable.expandedCandidates = []
stable.hotwordOverlay = nil; stable.candidatePanelExpanded = false
var visibleAndBlankDuringRestore = false
stable.duringAppContext = {
  visibleAndBlankDuringRestore = InputiaHost.candidatePanel!.visible && InputiaHost.candidatePanel!.shown.isEmpty
}
stable.clearManagedMemoryDisplay(restoringBasicCandidates: true)
precondition(visibleAndBlankDuringRestore && InputiaHost.candidatePanel!.events == ["blank", "show"],
  "相同基础候选恢复必须原窗清正文后替换，不能先hide")
checks += 1
// 同 controller 的新显示、或新 owner 的显示，都作废旧恢复闭包。
for switchOwner in [false, true] {
  InputiaHost.activeInputController = stable
  InputiaHost.candidatePanel = InputiaCandidatePanel()
  let restore = stable.managedMemoryWasCleared()
  precondition(InputiaHost.candidatePanel!.visible && InputiaHost.candidatePanel!.shown.isEmpty)
  if switchOwner { InputiaHost.activeInputController = newOwner }
  InputiaHost.candidatePanel!.show(candidates: ["新的安全候选"], near: .zero,
    expanded: false, primaryCandidateCount: 1, activeRowIndex: 0)
  let writes = InputiaHost.candidatePanel!.writes
  restore?()
  precondition(InputiaHost.candidatePanel!.writes == writes
    && InputiaHost.candidatePanel!.shown == ["新的安全候选"], "旧恢复不能重画或隐藏新显示")
  checks += 1
}
// 延迟恢复遇到失焦/安全输入/输入源改变时，空框也必须关闭。
for boundary in ["focus", "secure", "source"] {
  InputiaHost.activeInputController = stable
  InputiaHost.candidatePanel = InputiaCandidatePanel()
  stable.sourceSelected = true; secure = false
  let restore = stable.managedMemoryWasCleared()
  if boundary == "focus" { InputiaHost.activeInputController = newOwner }
  if boundary == "secure" { secure = true }
  if boundary == "source" { stable.sourceSelected = false }
  restore?()
  precondition(!InputiaHost.candidatePanel!.visible && InputiaHost.candidatePanel!.shown.isEmpty)
  checks += 1
}

// 最后的 attributes 查询也可能同步进入同 owner 的新输入；不能覆盖其新显示。
InputiaHost.activeInputController = stable
InputiaHost.candidatePanel = InputiaCandidatePanel()
stable.sourceSelected = true; secure = false
stable.live?.duringAttributes = {
  stable.latestCandidates = ["重入后的候选"]
  InputiaHost.candidatePanel!.show(candidates: stable.latestCandidates, near: .zero,
    expanded: false, primaryCandidateCount: 1, activeRowIndex: 0)
}
stable.clearManagedMemoryDisplay(restoringBasicCandidates: true)
precondition(InputiaHost.candidatePanel!.shown == ["重入后的候选"]
  && InputiaHost.candidatePanel!.events == ["blank", "show"], "最后IMK查询后仍须核显示token")
checks += 1

print("managedMemoryPanelContinuity=true oldTextClearedBeforeRestore=true noHideForFreshBase=true staleRestoreCannotTouchNewDisplay=true invalidBoundaryHides=true")

print("managedMemoryBasicFallbackSelfCheck=true checks=\(checks)")
'''

with tempfile.TemporaryDirectory(prefix="inputia-memory-basic-check-") as directory:
    swift = Path(directory) / "main.swift"
    binary = Path(directory) / "check"
    swift.write_text(fixture.replace("__ACTUAL_METHODS__", methods))
    subprocess.run(["swiftc", str(root / "Sources/InputiaInputMethod/InputiaConfirmedFieldContext.swift"), str(swift), "-o", str(binary)], check=True)
    subprocess.run([str(binary)], check=True)
