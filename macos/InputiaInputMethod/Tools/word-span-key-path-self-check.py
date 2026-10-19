#!/usr/bin/env python3
"""执行真实英文词段按键包装；仅用计数客户端替代 IMK，不启动窗口。"""
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
source = (root / "Sources/InputiaInputMethod/main.swift").read_text()

def extract(marker):
    start = source.index(marker)
    brace = source.index("{", start)
    end, depth = brace + 1, 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end].replace("private ", "")

methods = "\n".join(extract(marker) for marker in [
    "  private func wordSpanCaret(",
    "  private func wordSpanEdit(",
    "  private func handleWordSpanKeyDown(",
])
fixture = r'''
import AppKit
let keyCodeTab: UInt16 = 48, keyCodeReturn: UInt16 = 36, keyCodeKeypadEnter: UInt16 = 76, keyCodeEscape: UInt16 = 53
final class Client {
  var reads = 0
  var selection = NSRange(location: 3, length: 0), marked = NSRange(location: NSNotFound, length: 0)
  func selectedRange() -> NSRange { reads += 1; return selection }
  func markedRange() -> NSRange { reads += 1; return marked }
}
typealias IMKTextInput = Client
struct Outcome { var mode = "Chinese" }
final class Bridge { var latestOutcome = Outcome() }
enum InputiaWordSpanEdit { case tailBackspace(Int), append(String) }
enum InputiaWordSpanState { static func wordUnit(_ value: UInt8) -> Bool { true } }
final class WordSpan {
  var hasPermit = false, observations = 0
  func observed(_ edit: InputiaWordSpanEdit, before: UInt64, after: UInt64) { observations += 1 }
}
final class Controller {
  var bridge = Bridge(), wordSpan = WordSpan(), latestComposing = "", shiftEnglishComposition = ""
  var recallCandidates: [String] = [], personalPredictions: [String] = []
  var candidatePanelExpanded = false, wordSpanObservation = UUID(), baseCalls = 0, retireCalls = 0
  func retireWordSpan(reason: String) { retireCalls += 1; wordSpanObservation = UUID() }
  func prepareWordSpan(client: Client) {}
  func handleKeyDown(_ event: NSEvent, client: Client) -> Bool { baseCalls += 1; return true }
__METHODS__
}
func event(_ key: UInt16 = 0, text: String = "a", modifiers: NSEvent.ModifierFlags = []) -> NSEvent {
  NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: ProcessInfo.processInfo.systemUptime,
    windowNumber: 0, context: nil, characters: text, charactersIgnoringModifiers: text, isARepeat: false, keyCode: key)!
}
var checks = 0
for composing in ["", "ni", "nihao"] {
  let controller = Controller(), client = Client()
  controller.latestComposing = composing
  client.marked = NSRange(location: 0, length: composing.utf16.count)
  precondition(controller.handleWordSpanKeyDown(event(), client: client))
  precondition(client.reads == 0 && controller.baseCalls == 1,
    "普通中文不应因英文词段包装增加selectedRange/markedRange读取")
  checks += 1
}
for input in [event(), event(51, text: "") ] {
  let controller = Controller(), client = Client()
  controller.bridge.latestOutcome.mode = "English"; controller.wordSpan.hasPermit = true
  precondition(controller.handleWordSpanKeyDown(input, client: client))
  precondition(client.reads > 0 && controller.baseCalls == 1 && controller.wordSpan.observations == 1,
    "英文追加与尾部删除仍须走原caret证明和observed路径")
  checks += 1
}
let modified = Controller(), modifiedClient = Client()
modified.bridge.latestOutcome.mode = "English"
precondition(modified.handleWordSpanKeyDown(event(modifiers: [.command]), client: modifiedClient))
precondition(modifiedClient.reads == 0 && modified.baseCalls == 1)
checks += 1
print("wordSpanKeyPathSelfCheck=true checks=\(checks) chineseNoIMKQueries=true englishObservationPreserved=true")
'''
with tempfile.TemporaryDirectory(prefix="inputia-word-span-key-") as directory:
    swift = Path(directory) / "main.swift"
    binary = Path(directory) / "check"
    swift.write_text(fixture.replace("__METHODS__", methods))
    subprocess.run(["swiftc", str(swift), "-o", str(binary)], check=True)
    subprocess.run([str(binary)], check=True)
