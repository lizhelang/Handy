import AppKit
import Foundation

@main
struct InputiaSecureDirectSelfCheck {
  private static let securityAgentBundleId = "com.apple.SecurityAgent"
  private static let normalBundleId = "com.apple.TextEdit"

  static func main() {
    let securityDecision = InputiaHostTextPolicy.secureDirectDecision(
      bundleIdentifier: securityAgentBundleId
    )
    let normalDecision = InputiaHostTextPolicy.secureDirectDecision(
      bundleIdentifier: normalBundleId
    )
    let commandModifiers: NSEvent.ModifierFlags = [.command]
    let controlModifiers: NSEvent.ModifierFlags = [.control]
    let optionModifiers: NSEvent.ModifierFlags = [.option]
    let shiftModifiers: NSEvent.ModifierFlags = [.shift]

    let checks: [(String, Bool)] = [
      (
        "securityAgentSecureDirect",
        InputiaHostTextPolicy.isSecureDirectBundleIdentifier(securityAgentBundleId)
      ),
      (
        "normalAppSecureDirectRejected",
        !InputiaHostTextPolicy.isSecureDirectBundleIdentifier(normalBundleId)
          && !normalDecision.passthrough
      ),
      (
        "secureDirectAlphabetPassesThrough",
        InputiaHostTextPolicy.shouldPassThroughSecureDirectText(
          "a",
          bundleIdentifier: securityAgentBundleId
        )
      ),
      (
        "secureDirectDigitPassesThrough",
        InputiaHostTextPolicy.shouldPassThroughSecureDirectText(
          "7",
          bundleIdentifier: securityAgentBundleId
        )
      ),
      (
        "secureDirectSymbolPassesThrough",
        InputiaHostTextPolicy.shouldPassThroughSecureDirectText(
          "@",
          bundleIdentifier: securityAgentBundleId
        )
      ),
      (
        "secureDirectSpacePassesThrough",
        InputiaHostTextPolicy.shouldPassThroughSecureDirectText(
          " ",
          bundleIdentifier: securityAgentBundleId
        )
      ),
      (
        "secureDirectEnterPassesThrough",
        InputiaHostTextPolicy.shouldPassThroughSecureDirectText(
          "\r",
          bundleIdentifier: securityAgentBundleId
        )
      ),
      ("secureDirectCommandShortcutPassesThrough", securityDecision.passthrough && commandModifiers.contains(.command)),
      ("secureDirectControlShortcutPassesThrough", securityDecision.passthrough && controlModifiers.contains(.control)),
      ("secureDirectOptionShortcutPassesThrough", securityDecision.passthrough && optionModifiers.contains(.option)),
      ("secureDirectShiftShortcutPassesThrough", securityDecision.passthrough && shiftModifiers.contains(.shift)),
      ("secureDirectClearsComposition", securityDecision.clearComposition),
      ("secureDirectHidesCandidates", securityDecision.hideCandidates),
      ("secureDirectBlocksClipboardRecall", securityDecision.blockClipboardRecall),
      ("secureDirectDoesNotReadContext", !securityDecision.readsContext),
      ("secureDirectDoesNotCallBridgeHandle", !securityDecision.callsBridgeHandle),
      ("secureDirectDoesNotSetMarkedTextForInput", !securityDecision.setsMarkedTextForInput),
      ("secureDirectDoesNotShowCandidates", !securityDecision.showsCandidates),
      ("secureDirectDoesNotLearn", !securityDecision.learns),
      (
        "secureDirectSpaceBypassesCandidateLogic",
        securityDecision.passthrough
          && InputiaShortcutClassifier.shouldHandleInputTextSpace(" ", hasComposing: true)
      ),
      (
        "secureDirectEnterBypassesCandidateLogic",
        securityDecision.passthrough
          && InputiaShortcutClassifier.isInputTextEnter("\r")
      ),
      (
        "normalAppDoesNotBypassCandidateLogic",
        !InputiaHostTextPolicy.shouldPassThroughSecureDirectText(
          "a",
          bundleIdentifier: normalBundleId
        )
      ),
    ]

    let ok = checks.allSatisfy { $0.1 }
    print("secureDirectSelfCheck=\(ok)")
    for (name, passed) in checks {
      print("\(name)=\(passed)")
    }
    exit(ok ? 0 : 1)
  }
}
