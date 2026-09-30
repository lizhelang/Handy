#!/usr/bin/env python3
"""以替身宿主执行真实清理方法，不启动 App 或访问用户 profile。"""

from pathlib import Path
import subprocess
import tempfile


root = Path(__file__).resolve().parents[1]
source = (root / "Sources/InputiaInputMethod/main.swift").read_text()
start = source.index("  func clearManagedMemoryDisplay(")
end = source.index("  private func observePersonalKey(", start)
method = source[start:end]

fixture = r'''
import Foundation
struct InputiaBridgeOutcome {
  var ok = true, mode = "Chinese", composing = "ni", candidates = ["你", "呢"]
}
final class Client {
  var selection = NSRange(location: 2, length: 0)
  var marked = NSRange(location: 0, length: 2)
  func selectedRange() -> NSRange { selection }
  func markedRange() -> NSRange { marked }
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
final class Panel {
  var visible = true, shown: [String] = ["旧私密候选"]
  func hide() { visible = false; shown = [] }
}
enum InputiaHost {
  static var activeInputController: Controller?
  static var candidatePanel: Panel? = Panel()
}
final class InputiaPermissionLifecycle {
  static let shared = InputiaPermissionLifecycle()
  var isReady = true
}
var secure = false
func IsSecureEventInputEnabled() -> Bool { secure }
struct Target { let targetID = "old-target" }
enum InputiaVoiceInputLauncher { static func releaseTarget(_ id: String) {} }
struct Pending { let token = 1 }
struct Resettable {
  mutating func reset() {}
  mutating func cancel() {}
  mutating func finish(_ token: Int) -> Int? { token }
}
final class Controller {
  let bridge = Bridge()
  var live: Client? = Client(), sourceSelected = true
  var memoryViewGeneration: UInt64 = 1, personalRefreshGeneration: UInt64 = 1
  var memoryFieldID: String? = "old-field", memorySelectionIntent: Int? = 1
  var memoryDisplayedTargets = ["rank": Target()], memoryDisplayedComposing = ["rank": "ni"]
  var personalDeferredOrigin: Pending? = Pending()
  var personalInputDeferral = Resettable(), personalPhrase = Resettable(), personalization = Resettable()
  var personalRecoveryToken: Int? = 1, personalBoundaryProof: Int? = 1
  var pendingPersonalSelection: Int? = 1, personalUndo: Int? = 1
  var personalCandidates = ["秘密"], personalPredictions = ["秘密"], personalCode = "ni"
  var personalOrderLockedCode: String? = "ni"
  var sharedEnglishSelection = Resettable(), sharedChineseSelection = Resettable()
  var sharedChineseOrder: Int? = 1, hotwordOverlay: Int? = 1
  var sharedEnglishCandidates = ["secret": 1], explicitEnglishCandidates = ["secret": 1]
  var sharedEnglishRefreshQueued = true
  var latestComposing = "ni", latestCandidates = ["秘密"], expandedCandidates = ["秘密"]
  var expandedCandidateEntries = ["秘密"], expandedActiveRowIndex = 1
  var recallCandidates = ["秘密"], englishCompletionCandidates = ["secret"], englishCompletionPrefix = "sec"
  var candidatePanelExpanded = true, cachedAppContext: Int? = 1, pushedAppContext: Int? = 1
  func client() -> Client? { live }
  func retireWordSpan(reason: String) {}
  func retireMemoryCommit() {}
  func removeRetiredVoiceTarget(_ id: String) {}
  func isCurrentInputiaSourceSelected() -> Bool { sourceSelected }
  func updateCandidateWindow(client: Client) {
    InputiaHost.candidatePanel?.shown = latestCandidates
    InputiaHost.candidatePanel?.visible = true
  }
__ACTUAL_METHOD__
}
var checks = 0
func scenario(_ name: String, visible: Bool, restoring: Bool = true,
              configure: (Controller) -> Void = { _ in }) {
  let controller = Controller()
  InputiaHost.activeInputController = controller
  InputiaHost.candidatePanel = Panel()
  InputiaPermissionLifecycle.shared.isReady = true
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
  if !restoring { precondition(controller.bridge.clears == 0, name) }
  checks += 1
}
scenario("学习域不可用后仅恢复本次清理所得基础候选", visible: true)
scenario("CAPI失败不得复用旧ok结果", visible: false) { $0.bridge.clearSucceeds = false }
scenario("secure input", visible: false) { _ in secure = true }
scenario("权限撤销", visible: false) { _ in InputiaPermissionLifecycle.shared.isReady = false }
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
// 非活动控制器不能重画另一个控制器的候选窗。
let inactive = Controller()
InputiaHost.activeInputController = Controller()
InputiaHost.candidatePanel = Panel()
inactive.clearManagedMemoryDisplay(restoringBasicCandidates: true)
precondition(inactive.latestCandidates.isEmpty && InputiaHost.candidatePanel!.shown == ["旧私密候选"])
checks += 1
print("managedMemoryBasicFallbackSelfCheck=true checks=\(checks)")
'''

with tempfile.TemporaryDirectory(prefix="inputia-memory-basic-check-") as directory:
    swift = Path(directory) / "main.swift"
    binary = Path(directory) / "check"
    swift.write_text(fixture.replace("__ACTUAL_METHOD__", method))
    subprocess.run(["swiftc", str(swift), "-o", str(binary)], check=True)
    subprocess.run([str(binary)], check=True)
