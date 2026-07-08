import Darwin
import Foundation

@main
struct InputiaBridgePrivacySelfCheck {
  static func main() {
    let root = URL(fileURLWithPath: NSTemporaryDirectory())
      .appendingPathComponent(
        "InputiaBridgePrivacySelfCheck-\(ProcessInfo.processInfo.processIdentifier)",
        isDirectory: true
      )
    let result = InputiaRustBridge.debugClipboardPrivacySelfCheck(
      settingsPath: root.appendingPathComponent("settings.json").path
    )
    let checks: [(String, Bool)] = [
      ("texteditAllowsClipboardRead", result["textedit"] == true),
      ("onepasswordRejectsClipboardRead", result["onepassword"] == false),
      ("securityAgentRejectsClipboardRead", result["securityAgent"] == false),
      ("unknownRejectsClipboardRead", result["unknown"] == false),
      ("privateWindowRejectsClipboardRead", result["privateWindow"] == false),
    ]

    let ok = checks.allSatisfy { $0.1 }
    print("bridgePrivacySelfCheck=\(ok)")
    for (name, passed) in checks {
      print("\(name)=\(passed)")
    }
    exit(ok ? 0 : 1)
  }
}
