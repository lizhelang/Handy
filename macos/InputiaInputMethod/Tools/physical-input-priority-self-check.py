#!/usr/bin/env python3
"""抽取真实宿主方法验证物理输入优先；只使用合成事件、字段和传输。"""

import argparse
import ast
from pathlib import Path
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--main-source", type=Path, help="用指定旧源码运行行为失败对照")
args = parser.parse_args()

# 复用既有生命周期自检的替身与生产方法装配，不执行它的场景或编译入口。
lifecycle = ROOT / "Tools/prediction-lifecycle-integration-self-check.py"
tree = ast.parse(lifecycle.read_text(), filename=str(lifecycle))
setup = []
for node in tree.body:
    if isinstance(node, ast.With):
        break
    setup.append(node)
else:
    raise AssertionError("生命周期自检缺少独立编译入口，需更新本测试装配")
namespace = {"__file__": str(lifecycle), "__name__": "physical_input_fixture"}
original_argv = sys.argv
try:
    sys.argv = [str(lifecycle)]
    if args.main_source:
        sys.argv += ["--main-source", str(args.main_source)]
    exec(compile(ast.Module(body=setup, type_ignores=[]), str(lifecycle), "exec"), namespace)
finally:
    sys.argv = original_argv


def declaration(text: str, marker: str) -> str:
    start = text.index(marker)
    brace = text.index("{", start)
    depth, end = 1, brace + 1
    while depth and end < len(text):
        depth += (text[end] == "{") - (text[end] == "}")
        end += 1
    if depth:
        raise AssertionError(f"无法提取完整声明：{marker}")
    return text[start:end].replace("private ", "")


source = namespace["source"]
full_fixture = namespace["fixture"]
fixture = full_fixture.split("\nlet controller = Controller()\n", 1)[0]
methods = namespace["methods"]
for marker in [
    "  private func deferPersonalInput(",
    "  private func toggleInputModeFromShift(",
    "  private func handleFlagsChanged(",
    "  private func commitPendingChineseAsEnglish(",
    "  private func admitManagedSelection(",
]:
    methods += "\n" + declaration(source, marker)

# 计数器只观测真实 scope 方法被调用的次数，不替换其中的判定。
scope = declaration(source, "  private func deferredPersonalScopeMatches(")
methods = methods.replace(scope, scope.replace("{\n", "{\n    scopeChecks += 1\n", 1))
handler = declaration(source, "  override func handle(")
key_up = handler.split("    case .keyUp:\n", 1)[1].split("    default:\n", 1)[0]
methods += "\nfunc physicalKeyUp(_ event: NSEvent, client: Client) -> Bool {\n" + key_up + "\n}\n"
assert 'toggleInputModeFromShift(client: client, source: "deferred-keydown")' in declaration(
    source, "  private func handleKeyDown("
), "丢失 Shift 松开补偿必须仍经过统一切换入口"

fixture = fixture.replace("import AppKit\n", "import AppKit\nimport OSLog\nlet shiftDiagnostic = Logger(subsystem: \"InputiaSelfCheck\", category: \"physical-input\")\n", 1)
fixture = fixture.replace("  var selectionReads = 0", "  var selectionReads = 0, markedReads = 0, rangeTrapHits = 0\n  var forbidRangeRead = false")
fixture = fixture.replace("    selectionReads += 1", "    if forbidRangeRead { rangeTrapHits += 1 }\n    selectionReads += 1", 1)
fixture = fixture.replace("func markedRange() -> NSRange { marked }", "func markedRange() -> NSRange { markedReads += 1; if forbidRangeRead { rangeTrapHits += 1 }; return marked }")
fixture = fixture.replace("  var clears = 0", """  var clears = 0
  var modeShortcut = "shift", modeToggles = 0
  var willToggleMode: (() -> Void)?
  func inputModeToggleShortcut() -> String { modeShortcut }
  func toggleInputMode() -> InputiaBridgeOutcome {
    willToggleMode?()
    modeToggles += 1
    latestOutcome.mode = latestOutcome.mode == "Chinese" ? "English" : "Chinese"
    return latestOutcome
  }
""", 1)
fixture = fixture.replace("  func resetShiftInputModeSession(reason: String) {}", """  var shiftInputModeGesture = InputiaShortcutClassifier.ShiftInputModeGestureState()
  var shiftResets = 0, scopeChecks = 0
  var memoryTargetFields: [String: String] = [:], voiceStatus = ""
  func resetShiftInputModeSession(reason: String) { shiftResets += 1; shiftInputModeGesture.resetSession(modifiers: []) }
  func reloadSettingsIfDue(client: Client) {}
  func clearEnglishCompletion() {}
  func prepareWordSpan(client: Client) {}
  // 此处替换显示租约边界；准入、token 和回调仍由真实方法执行。
  func managedDisplayCurrent(_ kind: String, client: Client) -> Bool { true }
""")
memory_source = (ROOT / "Sources/InputiaInputMethod/InputiaManagedMemory.swift").read_text()
memory_types = declaration(memory_source, "struct InputiaMemoryTarget:") + "\n" + declaration(memory_source, "enum InputiaMemorySelectionAdmission")
memory_types += """
extension InputiaMemoryTarget {
  init(_ target: InputiaVoiceTarget) {
    self.init(target_id: target.target_id, host_instance: target.host_instance,
      controller_id: target.controller_id, activation_generation: target.activation_generation,
      field_id: target.field_id, selection_generation: target.selection_generation,
      composition_generation: target.composition_generation, source_app: target.source_app)
  }
}
"""
fixture += "\n" + memory_types + "\n"
for marker in ["func takeQuery()", "func response(", "func openedQuery()", "func takeCommand(", "func admitSelected("]:
    fixture += declaration(full_fixture, marker) + "\n"

fixture += r'''
let completed = InputiaBridgeOutcome(composing: "", candidates: [], commit: "你好", candidateIDs: [])
var failures = 0, checks = 0
func check(_ passed: Bool, _ label: String) {
  checks += 1
  if !passed { failures += 1; print("FAIL \(label)") }
}
func physicalEvent(_ type: NSEvent.EventType, code: UInt16, text: String = "", modifiers: NSEvent.ModifierFlags = []) -> NSEvent {
  NSEvent.keyEvent(with: type, location: .zero, modifierFlags: modifiers,
    timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: 0, context: nil,
    characters: text, charactersIgnoringModifiers: text, isARepeat: false, keyCode: code)!
}
func barePending() -> (Controller, UInt64) {
  let value = Controller()
  InputiaHost.activeInputController = value
  let target = InputiaVoiceTarget(target_id: UUID().uuidString)
  let origin = InputiaVoiceTargetSnapshot.Snapshot(target)
  value.typedCompositionOrigin = origin
  let token = value.personalInputDeferral.begin(now: ProcessInfo.processInfo.systemUptime)!
  value.personalDeferredOrigin = .init(token: token, origin: origin, client: value.live,
    activation: value.voiceActivationGeneration, generation: value.localSelectionGeneration,
    epoch: value.personalization.epoch, schemaID: value.bridge.schemaID, code: value.latestComposing,
    selection: value.live.selection, marked: value.live.marked, managedField: nil)
  return (value, token)
}
func stop(_ value: Controller) {
  if let pending = value.personalDeferredOrigin { value.finishDeferredPersonalInput(pending.token) }
  value.personalization.stop()
  value.closePredictionPresentation()
}
func admittedPending() -> (Controller, InputiaVoiceInputLauncher.Pending) {
  let (value, query) = openedQuery()
  query.completion(response(query, text: "合成待选词"))
  precondition(value.handle(event(), client: value.live), "真实推荐选择必须开始准入")
  return (value, takeCommand("admit"))
}

// 命令和普通新字都必须返回当前 IMK 路径，不能先消费再丢弃。
for (label, code, text, modifiers) in [
  ("Cmd+C", UInt16(8), "c", NSEvent.ModifierFlags.command),
  ("Cmd+V", 9, "v", .command), ("Cmd+X", 7, "x", .command),
  ("Cmd+A", 0, "a", .command), ("Cmd+Z", 6, "z", .command),
  ("普通新字", 0, "a", []),
] {
  let (value, _) = barePending()
  value.live.selectionReads = 0; value.live.markedReads = 0
  let before = ProcessInfo.processInfo.systemUptime
  let result = value.deferPersonalInput(event(code, text: text, modifiers: modifiers), client: value.live)
  let elapsed = ProcessInfo.processInfo.systemUptime - before
  check(result == nil && !value.personalPredictionPending && value.personalInputDeferral.events.isEmpty,
    "\(label)：新按键取消旧选择并返回 nil，不排队、不吞键")
  check(value.scopeChecks == 0 && value.live.selectionReads == 0 && value.live.markedReads == 0,
    "\(label)：无证明取消不探测 scope 或范围")
  check(elapsed < 0.1 && value.shiftResets == 0, "\(label)：不等待 750ms，不重置真实 Shift 手势")
  stop(value)
}
for (label, input) in [
  ("选择键松开", physicalEvent(.keyUp, code: 49)),
  ("Command 按下", physicalEvent(.flagsChanged, code: 55, modifiers: [.command])),
  ("Command 松开", physicalEvent(.flagsChanged, code: 55)),
  ("左右 Command 重叠释放", physicalEvent(.flagsChanged, code: 55, modifiers: [.command])),
  ("Option 松开", physicalEvent(.flagsChanged, code: 58)),
  ("Shift 松开", physicalEvent(.flagsChanged, code: 56)),
] {
  let (value, token) = barePending()
  check(value.deferPersonalInput(input, client: value.live) == nil
    && value.personalInputDeferral.token == token && value.personalInputDeferral.events.isEmpty,
    "\(label)：直接放行并保留待选 token")
  stop(value)
}

// 重叠 Shift 的第一次释放不是模式切换；最后一次真实释放才取消旧选择。
for firstShift: UInt16 in [56, 60] {
  let (value, token) = barePending()
  value.latestComposing = ""; value.bridge.latestOutcome.composing = ""
  value.bridge.willToggleMode = { [weak value] in
    check(value?.personalPredictionPending == false, "重叠 Shift 模式执行前已清除旧 token")
  }
  let secondShift: UInt16 = firstShift == 56 ? 60 : 56
  for (code, flags) in [(firstShift, NSEvent.ModifierFlags.shift), (secondShift, .shift), (firstShift, .shift)] {
    let input = physicalEvent(.flagsChanged, code: code, modifiers: flags)
    _ = value.deferPersonalInput(input, client: value.live)
    _ = value.handleFlagsChanged(input, client: value.live)
  }
  check(value.personalInputDeferral.token == token && value.bridge.modeToggles == 0,
    "重叠 Shift 首先释放 \(firstShift) 不取消选择或切换")
  let released = physicalEvent(.flagsChanged, code: secondShift)
  _ = value.deferPersonalInput(released, client: value.live)
  _ = value.handleFlagsChanged(released, client: value.live)
  check(!value.personalPredictionPending && value.bridge.modeToggles == 1 && value.shiftResets == 0,
    "重叠 Shift 最后释放 \(secondShift) 真实切换前取消选择")
  stop(value)
}
for source in ["physical-keyup", "deferred-keydown"] {
  let (value, _) = barePending()
  value.latestComposing = ""; value.bridge.latestOutcome.composing = ""
  value.bridge.willToggleMode = { [weak value] in
    check(value?.personalPredictionPending == false, "Shift \(source) 模式执行前已清除旧 token")
  }
  if source == "physical-keyup" {
    _ = value.handleFlagsChanged(physicalEvent(.flagsChanged, code: 56, modifiers: [.shift]), client: value.live)
    _ = value.physicalKeyUp(physicalEvent(.keyUp, code: 56), client: value.live)
  } else {
    _ = value.toggleInputModeFromShift(client: value.live, source: source)
  }
  check(!value.personalPredictionPending && value.bridge.modeToggles == 1 && value.shiftResets == 0,
    "Shift \(source) 经统一入口取消且不重置手势")
  stop(value)
}

// 命令/明确编辑只清自有上下文，不能在放行前询问宿主范围。
for (label, input) in [
  ("Cmd+C", event(8, text: "c", modifiers: [.command])),
  ("Cmd+V", event(9, text: "v", modifiers: [.command])),
  ("左箭头", event(123, text: "")),
  ("Shift 边界", physicalEvent(.flagsChanged, code: 56, modifiers: [.shift])),
] {
  let value = Controller(); InputiaHost.activeInputController = value
  value.live.selection = NSRange(location: 2, length: 0); value.live.marked = emptyReplacementRange
  value.confirmedFieldContext.committed("合成", start: 0, selection: value.live.selection, scope: value.confirmedFieldScope(value.live))
  value.live.forbidRangeRead = true
  value.observeConfirmedFieldInput(input, client: value.live)
  check(!value.confirmedFieldContext.hasContext && value.live.rangeTrapHits == 0
    && value.live.selectionReads == 0 && value.live.markedReads == 0,
    "\(label)：真实 observer 在范围陷阱前清上下文")
  stop(value)
}
for moved in [false, true] {
  let value = Controller(); InputiaHost.activeInputController = value
  value.live.selection = NSRange(location: 2, length: 0); value.live.marked = emptyReplacementRange
  value.confirmedFieldContext.committed("合成", start: 0, selection: value.live.selection, scope: value.confirmedFieldScope(value.live))
  if moved { value.live.selection.location = 3 }
  value.observeConfirmedFieldInput(event(0, text: "a"), client: value.live)
  check(value.live.selectionReads == 1 && value.live.markedReads == 1
    && value.confirmedFieldContext.hasContext == !moved,
    "普通字仍核验范围，范围改变=\(moved) 时清上下文")
  stop(value)
}

// 真实 admit/validate 回调必须先拒绝已取消 token，不能插入或补发后续验证。
for phase in ["admit", "validate"] {
  let (value, admission) = admittedPending()
  let before = value.live.inserts
  var validate: ((InputiaTargetBridgeReply?) -> Void)?
  if phase == "validate" { admitSelected(admission); validate = InputiaVoiceInputLauncher.targetReplies.removeFirst() }
  _ = value.deferPersonalInput(event(9, text: "v", modifiers: [.command]), client: value.live)
  let targetRequests = InputiaVoiceInputLauncher.targetReplies.count
  if let validate { validate(.init(target: admission.command.target)) } else { admitSelected(admission) }
  check(value.live.inserts == before && !value.personalPredictionPending
    && InputiaVoiceInputLauncher.targetReplies.count == targetRequests,
    "真实迟到 \(phase) 回调被 token 拒绝")
  stop(value)
}

// 受管候选也走真实 admitManagedSelection 回调，perform 替身仅计数。
for cancelled in [false, true] {
  let value = Controller(); InputiaHost.activeInputController = value
  let origin = InputiaVoiceTargetSnapshot.Snapshot(.init(target_id: UUID().uuidString))
  value.memoryDisplayedTargets["rank"] = origin
  value.memoryTargetFields[origin.targetID] = "service:field-A"; value.memoryFieldID = "service:field-A"
  var performed = 0
  precondition(value.admitManagedSelection("rank", client: value.live, stillValid: { true }, perform: { performed += 1 }))
  let reply = InputiaVoiceInputLauncher.targetReplies.removeFirst()
  if cancelled { _ = value.deferPersonalInput(event(8, text: "c", modifiers: [.command]), client: value.live) }
  reply(.init(target: origin.inputiaTarget, field_instance: "field-A"))
  check(performed == (cancelled ? 0 : 1), "真实受管 perform：取消=\(cancelled)")
  stop(value)
}

// 旧超时闭包和旧完成 token 不能清理较新的一轮。
let (timed, oldAdmission) = admittedPending()
let oldToken = timed.personalInputDeferral.token!
_ = timed.deferPersonalInput(event(0, text: "a"), client: timed.live)
check(!timed.personalPredictionPending, "旧超时场景：新字必须先取消旧选择")
// 旧源对照可能仍保留旧 token；显式收尾后继续检查，避免用 fixture 崩溃冒充回归失败。
if timed.personalPredictionPending { timed.finishDeferredPersonalInput(oldToken) }
pump(0.36)
let origin = timed.typedCompositionOrigin!
let newToken = timed.personalInputDeferral.begin(now: ProcessInfo.processInfo.systemUptime)!
timed.personalDeferredOrigin = .init(token: newToken, origin: origin, client: timed.live,
  activation: timed.voiceActivationGeneration, generation: timed.localSelectionGeneration,
  epoch: timed.personalization.epoch, schemaID: timed.bridge.schemaID, code: timed.latestComposing,
  selection: timed.live.selection, marked: timed.live.marked, managedField: nil)
let insertsBeforeTimeout = timed.live.inserts
timed.finishDeferredPersonalInput(oldToken)
admitSelected(oldAdmission)
pump(0.45)
check(timed.personalInputDeferral.token == newToken && timed.live.inserts == insertsBeforeTimeout,
  "真实旧超时/旧完成/迟到 admit 不能清新 token 或插入")
stop(timed)
print("physicalInputPrioritySelfCheck=\(failures == 0) checks=\(checks) failures=\(failures) productionMethods=true realAdmissionCallbacks=true fakeNativeBoundary=true nativeIMK=false")
exit(failures == 0 ? 0 : 1)
'''

query_fields = """
  var predictionQueryContext: PredictionQueryContext?
  var predictionTarget: InputiaVoiceTargetSnapshot.Snapshot?
  var predictionTargetCapture: UUID?
  var predictionQueryGeneration: UInt64 = 0, predictionActivationGeneration: UInt64 = 0
"""
fixture = (fixture.replace("__METHODS__", methods)
    .replace("__PREDICTION_QUERY_FIELDS__", query_fields)
    .replace("__CALLBACKS__", namespace["callbacks"])
    .replace("__CAPTURE_ANCHOR_START__", namespace["capture_anchor_start"])
    .replace("__CAPTURE_ANCHOR_FINISH__", namespace["capture_anchor_finish"]))

with tempfile.TemporaryDirectory(prefix="inputia-physical-input-priority-") as temporary:
    directory = Path(temporary)
    swift = directory / "main.swift"
    swift.write_text(fixture)
    binary = directory / "check"
    subprocess.run([
        "/usr/bin/swiftc", "-D", "INPUTIA_PAIRED_BUILD", "-D", "PREDICTION_QUERY_CONTEXT",
        "-D", "PREDICTION_ACCEPT_CONTEXT", "-D", "PREDICTION_KEYS",
        str(ROOT / "Sources/InputiaInputMethod/InputiaPredictionPresentation.swift"),
        str(ROOT / "Sources/InputiaInputMethod/InputiaConfirmedFieldContext.swift"),
        str(ROOT / "Sources/InputiaInputMethod/InputiaShortcutClassifier.swift"),
        str(ROOT / "Sources/InputiaInputMethod/InputiaPersonalization.swift"),
        str(swift), "-o", str(binary),
    ], check=True)
    raise SystemExit(subprocess.run([str(binary)], check=False).returncode)
