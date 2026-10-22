#!/usr/bin/env python3
"""执行真实 Return 分支和上屏方法，验证确认组合的回车不会同时交给宿主。"""

import argparse
from pathlib import Path
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--main-source", type=Path, help="指定旧宿主源码作运行失败对照")
parser.add_argument("--policy-source", type=Path, help="与旧宿主源码配套的文本策略文件")
args = parser.parse_args()
source = (args.main_source or ROOT / "Sources/InputiaInputMethod/main.swift").read_text()
policy = args.policy_source or ROOT / "Sources/InputiaInputMethod/InputiaHostTextPolicy.swift"


def declaration(marker):
    start = source.index(marker)
    brace = source.index("{", start)
    end, depth = brace + 1, 1
    while depth and end < len(source):
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    if depth:
        raise AssertionError(f"无法提取完整生产声明：{marker}")
    return source[start:end].replace("private ", "")


key_down = declaration("  private func handleKeyDown(")
return_branch = key_down.split("    case keyCodeReturn, keyCodeKeypadEnter:\n", 1)[1].split(
    "    case keyCodeSpace:\n", 1
)[0]
modifier_gate = declaration(
    "    if modifiers.contains(.command) || modifiers.contains(.control) || modifiers.contains(.option)"
)
assert key_down.index(modifier_gate) < key_down.index("    case keyCodeReturn, keyCodeKeypadEnter:"), (
    "宿主快捷键必须先于 Return 组合处理放行"
)
assert "if let handled = handlePersonalCandidateKey(event, client: client) { return handled }" in key_down
# 延后上屏只有补完路径齐全才不会丢字：任何新事件、外部提交和停用都必须先补完。
handle = declaration("  override func handle(_ event: NSEvent!, client sender: Any!)")
flush_gate = "    if pendingReturnCommit != nil {\n      flushPendingReturnCommit()"
assert flush_gate in handle and handle.index(flush_gate) < handle.index("shouldUseSecureDirectMode(client)"), (
    "handle 必须在任何其他处理之前补完延后的回车上屏"
)
for marker in ["  override func commitComposition(_ sender: Any!)", "  override func deactivateServer(_ sender: Any!)"]:
    body = declaration(marker)
    assert body.split("{", 1)[1].lstrip().startswith("flushPendingReturnCommit()"), f"{marker.strip()} 必须先补完上屏"
methods = "\n".join(
    declaration(marker)
    for marker in [
        "  private func apply(",
        "  private func commitComposingOnReturn(",
        "  private func deferReturnCommit(",
        "  private func flushPendingReturnCommit(",
        "  private func syncHostState(",
        "  private func insertCommittedText(",
        "  private func clearShiftEnglishComposition(",
        "  private func commitShiftEnglishComposition(",
        "  private func predictionSelectionIndex(",
        "  private func handlePersonalCandidateKey(",
    ]
)

# 只替换引擎结果、客户端和显示/记录边界；Return 判断、实际 insertText 调用及
# apply 的 consumed 返回值均从生产源码提取。配对学习等副作用不在本检查范围内。
fixture = r'''
import AppKit
import Foundation
let keyCodeReturn: UInt16 = 36, keyCodeKeypadEnter: UInt16 = 76
let keyCodeSpace: UInt16 = 49, keyCodeTab: UInt16 = 48
let emptyReplacementRange = NSRange(location: NSNotFound, length: NSNotFound)
func inputiaDebugLog(_ value: String) {}
struct PredictionCommitTrigger {}
struct InputiaBridgeOutcome {
  var ok = true, consumed = true, mode = "Chinese", composing = ""
  var candidates: [String] = [], commit: String? = nil
}
final class Bridge {
  var outcome = InputiaBridgeOutcome(), calls = 0
  func enter() -> InputiaBridgeOutcome { calls += 1; return outcome }
}
final class Client {
  var inserted: [String] = []
  var marked = NSRange(location: 0, length: 3)
  var bundle: String? = "com.apple.TextEdit"
  func bundleIdentifier() -> String? { bundle }
  func markedRange() -> NSRange { marked }
  func insertText(_ value: String, replacementRange: NSRange) { inserted.append(value); marked = emptyReplacementRange }
}
typealias IMKTextInput = Client
final class Panel { func hide() {} }
final class Controller {
  let bridge = Bridge()
  var pendingReturnCommit: (() -> Void)?
  var pendingReturnCommitGeneration: UInt64 = 0
  var latestComposing = "", latestCandidates: [String] = [], shiftEnglishComposition = ""
  var englishCompletionPrefix = "", englishCompletionCandidates: [String] = []
  var candidatePanelExpanded = false, expandedActiveRowIndex = 0
  var expandedCandidates: [String] = [], expandedCandidateEntries: [String] = []
  var ownedCandidatePanel: Panel? = Panel()
  var personalInputReplayDepth = 0, personalPredictions: [String] = []
  var predictionDisplayedAt: TimeInterval? = nil, predictionAccepts = 0
  var completionClears = 0
  func cancelShiftInputModeGesture(reason: String) {}
  func clearClipboardRecall() {}
  func clearEnglishCompletion() { englishCompletionCandidates = []; completionClears += 1 }
  func learnAndClearEnglishCompletion(client: Client) { clearEnglishCompletion() }
  func clearMarkedText(_ client: Client) { client.marked = emptyReplacementRange }
  func setMarkedComposition(_ text: String, client: Client) { client.marked = NSRange(location: 0, length: text.utf16.count) }
  func updateCandidateWindow(client: Client) {}
  func acceptPersonalPrediction(_ text: String, client: Client) -> Bool { predictionAccepts += 1; return true }
  func handleReturn(_ event: NSEvent, client: Client) -> Bool {
    let modifiers = event.modifierFlags.intersection(.deviceIndependentFlagsMask)
__MODIFIER_GATE__
    if let handled = handlePersonalCandidateKey(event, client: client) { return handled }
    switch event.keyCode {
    case keyCodeReturn, keyCodeKeypadEnter:
__RETURN_BRANCH__
    default: return false
    }
  }
__METHODS__
}
func event(_ code: UInt16, modifiers: NSEvent.ModifierFlags = [], repeat: Bool = false) -> NSEvent {
  NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: modifiers,
    timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: 0, context: nil,
    characters: "\r", charactersIgnoringModifiers: "\r", isARepeat: `repeat`, keyCode: code)!
}
let webApp = "com.apple.Safari.WebApp.7DAA0D15-0997-433C-9820-ED5C74751C09"
var checks = 0, failures = 0
func check(_ passed: Bool, _ label: String) {
  checks += 1
  if !passed { failures += 1; print("FAIL \(label)") }
}
for code: UInt16 in [36, 76] {
  for (label, composing, candidates, commit) in [
    ("ASCII 原文无候选", "abc", [String](), "abc"),
    ("ASCII 含空格符号", "a@b.com", [], "a@b.com"),
    ("拼音有原生候选", "ni", ["你"], "ni"),
    ("非 ASCII 原文", "你好", [], "你好"),
    ("引擎提交不同文本", "abc", [], "ab"),
  ] {
    let value = Controller(), client = Client()
    value.latestComposing = composing; value.latestCandidates = candidates
    value.bridge.outcome.commit = commit
    let consumed = value.handleReturn(event(code), client: client)
    check(consumed, "\(code) \(label)：上屏的同一次回车必须被消费")
    check(client.inserted == [commit] && value.latestComposing.isEmpty && value.bridge.calls == 1,
      "\(code) \(label)：只插入一次并清空组合")
    check(!value.handleReturn(event(code), client: client) && client.inserted == [commit]
      && value.bridge.calls == 1, "\(code) \(label)：下一次无组合回车交给宿主且不重复插入")
  }
  for mode in ["Chinese", "English"] {
    let value = Controller(), client = Client()
    value.bridge.outcome.mode = mode; value.englishCompletionCandidates = ["hello"]
    check(!value.handleReturn(event(code), client: client) && client.inserted.isEmpty
      && value.bridge.calls == 0 && value.englishCompletionCandidates.isEmpty,
      "\(code) \(mode)：无组合回车保留宿主行为")
  }
  for modifiers: NSEvent.ModifierFlags in [.command, .control, .option, [.command, .shift]] {
    let value = Controller(), client = Client()
    value.latestComposing = "abc"; value.bridge.outcome.commit = "abc"
    check(!value.handleReturn(event(code, modifiers: modifiers), client: client)
      && client.inserted.isEmpty && value.latestComposing == "abc" && value.bridge.calls == 0,
      "\(code) 修饰快捷键保留宿主行为")
  }
  let shifted = Controller(), shiftedClient = Client()
  shifted.shiftEnglishComposition = "AbC"; shifted.latestComposing = "AbC"
  check(shifted.handleReturn(event(code), client: shiftedClient)
    && shiftedClient.inserted == ["AbC"] && shifted.latestComposing.isEmpty
    && shifted.shiftEnglishComposition.isEmpty && shifted.bridge.calls == 0,
    "\(code) Shift 英文组合仍只上屏一次")
  check(!shifted.handleReturn(event(code), client: shiftedClient) && shiftedClient.inserted == ["AbC"],
    "\(code) Shift 英文上屏后的下一次回车交给宿主")
  let prediction = Controller(), predictionClient = Client()
  prediction.personalPredictions = ["联想词"]
  prediction.predictionDisplayedAt = ProcessInfo.processInfo.systemUptime - 1
  check(!prediction.handleReturn(event(code), client: predictionClient)
    && prediction.predictionAccepts == 0 && predictionClient.inserted.isEmpty,
    "\(code) 空输入联想不能误吞 Return 或选词")
  // 程序坞网页应用：keyDown 只吞下并保留组合，补完（同一回车 keyUp）时才上屏一次。
  let deferred = Controller(), deferredClient = Client()
  deferredClient.bundle = webApp
  deferred.latestComposing = "nihk"; deferred.bridge.outcome.commit = "nihk"
  check(deferred.handleReturn(event(code), client: deferredClient) && deferredClient.inserted.isEmpty
    && deferred.latestComposing == "nihk" && deferred.bridge.calls == 0 && deferred.pendingReturnCommit != nil,
    "\(code) 网页应用：回车 keyDown 吞下且组合仍在进行")
  deferred.flushPendingReturnCommit()
  check(deferredClient.inserted == ["nihk"] && deferred.latestComposing.isEmpty && deferred.bridge.calls == 1
    && deferred.pendingReturnCommit == nil, "\(code) 网页应用：keyUp 补完后只上屏一次")
  deferred.flushPendingReturnCommit()
  check(!deferred.handleReturn(event(code), client: deferredClient) && deferredClient.inserted == ["nihk"]
    && deferred.bridge.calls == 1, "\(code) 网页应用：重复补完无副作用，下一次无组合回车交给宿主")
  let deferredShift = Controller(), deferredShiftClient = Client()
  deferredShiftClient.bundle = webApp
  deferredShift.shiftEnglishComposition = "AbC"; deferredShift.latestComposing = "AbC"
  check(deferredShift.handleReturn(event(code), client: deferredShiftClient)
    && deferredShiftClient.inserted.isEmpty && deferredShift.shiftEnglishComposition == "AbC",
    "\(code) 网页应用：Shift 英文组合同样等 keyUp")
  deferredShift.flushPendingReturnCommit()
  check(deferredShiftClient.inserted == ["AbC"] && deferredShift.shiftEnglishComposition.isEmpty,
    "\(code) 网页应用：Shift 英文补完后只上屏一次")
  // 该宿主不送回车 keyUp：没有任何后续事件时也必须由延时自行上屏，候选随之消失。
  let unattended = Controller(), unattendedClient = Client()
  unattendedClient.bundle = webApp
  unattended.latestComposing = "nihk"; unattended.bridge.outcome.commit = "nihk"
  check(unattended.handleReturn(event(code), client: unattendedClient) && unattendedClient.inserted.isEmpty,
    "\(code) 网页应用：吞下回车时尚未上屏")
  RunLoop.main.run(until: Date().addingTimeInterval(InputiaHostTextPolicy.deferredReturnCommitDelay + 0.2))
  check(unattendedClient.inserted == ["nihk"] && unattended.latestComposing.isEmpty
    && unattended.pendingReturnCommit == nil && unattended.bridge.calls == 1,
    "\(code) 网页应用：无 keyUp 也在延时后只上屏一次")
  check(!unattended.handleReturn(event(code), client: unattendedClient) && unattendedClient.inserted == ["nihk"],
    "\(code) 网页应用：自动上屏后的下一次回车交给宿主")
  // 延时到期前来了新事件：handle 开头先补完（此处按生产顺序直接调用），之后新的组合再次
  // 延后；旧代次的延时不能重复上屏，两次各上屏一次且顺序不变。
  let chained = Controller(), chainedClient = Client()
  chainedClient.bundle = webApp
  chained.latestComposing = "abc"; chained.bridge.outcome.commit = "abc"
  _ = chained.handleReturn(event(code), client: chainedClient)
  chained.flushPendingReturnCommit()
  check(chainedClient.inserted == ["abc"] && chained.latestComposing.isEmpty,
    "\(code) 网页应用：新事件到来先补完前一次")
  chained.latestComposing = "def"; chained.bridge.outcome.commit = "def"
  check(chained.handleReturn(event(code), client: chainedClient) && chainedClient.inserted == ["abc"],
    "\(code) 网页应用：新的组合回车再次延后")
  RunLoop.main.run(until: Date().addingTimeInterval(InputiaHostTextPolicy.deferredReturnCommitDelay + 0.2))
  check(chainedClient.inserted == ["abc", "def"] && chained.pendingReturnCommit == nil
    && chained.bridge.calls == 2, "\(code) 网页应用：两次各只上屏一次且顺序不变")
  let repeated = Controller(), repeatedClient = Client()
  repeatedClient.bundle = webApp
  repeated.latestComposing = "abc"; repeated.bridge.outcome.commit = "abc"
  check(repeated.handleReturn(event(code, repeat: true), client: repeatedClient)
    && repeatedClient.inserted == ["abc"] && repeated.pendingReturnCommit == nil,
    "\(code) 网页应用：按住重复的回车不进入延后")
  let failed = Controller(), failedClient = Client()
  failed.latestComposing = "abc"; failed.bridge.outcome.ok = false
  check(!failed.handleReturn(event(code), client: failedClient) && failedClient.inserted.isEmpty,
    "\(code) 引擎失败不能伪称已上屏")
}
print("returnKeyCommitSelfCheck=\(failures == 0) checks=\(checks) failures=\(failures)")
exit(failures == 0 ? 0 : 1)
'''

with tempfile.TemporaryDirectory(prefix="inputia-return-key-") as directory:
    swift = Path(directory) / "main.swift"
    binary = Path(directory) / "check"
    swift.write_text(
        fixture.replace("__MODIFIER_GATE__", modifier_gate)
        .replace("__RETURN_BRANCH__", return_branch)
        .replace("__METHODS__", methods)
    )
    subprocess.run(["/usr/bin/swiftc", str(swift), str(policy), "-o", str(binary)], check=True)
    subprocess.run([str(binary)], check=True)
