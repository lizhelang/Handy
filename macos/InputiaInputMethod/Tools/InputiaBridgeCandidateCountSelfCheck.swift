import Foundation

@main
enum InputiaBridgeCandidateCountSelfCheck {
  static func main() {
    let root = URL(fileURLWithPath: NSTemporaryDirectory())
      .appendingPathComponent(
        "InputiaBridgeCandidateCountSelfCheck-\(ProcessInfo.processInfo.processIdentifier)",
        isDirectory: true
      )
    let outcome = InputiaRustBridge.debugCandidateCountFallbackSelfCheck(
      settingsPath: root.appendingPathComponent("settings.json").path
    )
    let passed = outcome.mode == "Chinese"
      && outcome.composing == "yh"
      && outcome.candidates.count == 8
      && outcome.candidates.contains("洋")
    print("candidateCountFallbackSelfCheckPassed=\(passed)")
    print("mode=\(outcome.mode)")
    print("composing=\(outcome.composing)")
    print("candidateCount=\(outcome.candidates.count)")
    print("candidates=\(outcome.candidates.joined(separator: ","))")
    if !passed {
      exit(1)
    }
  }
}
