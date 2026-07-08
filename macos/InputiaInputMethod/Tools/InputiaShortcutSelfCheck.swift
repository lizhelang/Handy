import AppKit
import Darwin

@main
struct InputiaShortcutSelfCheck {
  private static let keyCodeSpace: UInt16 = 49
  private static let keyCodePeriod: UInt16 = 47
  private static let keyCodeDownArrow: UInt16 = 125
  private static let keyCodeUpArrow: UInt16 = 126
  private struct CommonCommandShortcut {
    let name: String
    let keyCode: UInt16
    let modifiers: NSEvent.ModifierFlags
  }
  private static let commonCommandShortcutNames = [
    "commandCPassThrough",
    "commandVPassThrough",
    "commandShiftVPassThrough",
    "commandOptionVPassThrough",
    "commandOptionShiftVPassThrough",
    "commandControlVPassThrough",
    "commandXPassThrough",
    "commandZPassThrough",
    "commandShiftZPassThrough",
    "commandAPassThrough",
    "commandSPassThrough",
    "commandOPassThrough",
    "commandWPassThrough",
    "commandQPassThrough",
    "commandFPassThrough",
    "commandGPassThrough",
    "commandHPassThrough",
    "commandMPassThrough",
    "commandPPassThrough",
    "commandTPassThrough",
    "commandNPassThrough",
    "commandDPassThrough",
    "commandEPassThrough",
    "commandIPassThrough",
    "commandRPassThrough",
    "commandJPassThrough",
    "commandKPassThrough",
    "commandYPassThrough",
    "commandCommaPassThrough",
    "commandTabPassThrough",
    "commandSpacePassThrough",
    "commandNumberPassThrough",
    "commandBracketPassThrough",
    "commandArrowPassThrough",
    "commandDeletePassThrough",
    "commandControlQPassThrough",
    "commandShift3PassThrough",
    "commandShift4PassThrough",
    "commandShift5PassThrough",
    "commandOptionEscapePassThrough",
  ]
  private static let officialAppleCommandKeyDownShortcuts: [CommonCommandShortcut] = [
    CommonCommandShortcut(name: "appleCommandCKeyDownPassThrough", keyCode: 8, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandVKeyDownPassThrough", keyCode: 9, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandXKeyDownPassThrough", keyCode: 7, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandZKeyDownPassThrough", keyCode: 6, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandShiftZKeyDownPassThrough", keyCode: 6, modifiers: [.command, .shift]),
    CommonCommandShortcut(name: "appleCommandAKeyDownPassThrough", keyCode: 0, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandFKeyDownPassThrough", keyCode: 3, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandGKeyDownPassThrough", keyCode: 5, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandShiftGKeyDownPassThrough", keyCode: 5, modifiers: [.command, .shift]),
    CommonCommandShortcut(name: "appleCommandHKeyDownPassThrough", keyCode: 4, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandOptionHKeyDownPassThrough", keyCode: 4, modifiers: [.command, .option]),
    CommonCommandShortcut(name: "appleCommandMKeyDownPassThrough", keyCode: 46, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandOptionMKeyDownPassThrough", keyCode: 46, modifiers: [.command, .option]),
    CommonCommandShortcut(name: "appleCommandNKeyDownPassThrough", keyCode: 45, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandOKeyDownPassThrough", keyCode: 31, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandPKeyDownPassThrough", keyCode: 35, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandSKeyDownPassThrough", keyCode: 1, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandWKeyDownPassThrough", keyCode: 13, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandQKeyDownPassThrough", keyCode: 12, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandBKeyDownPassThrough", keyCode: 11, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandIKeyDownPassThrough", keyCode: 34, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandUKeyDownPassThrough", keyCode: 32, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandKKeyDownPassThrough", keyCode: 40, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandLKeyDownPassThrough", keyCode: 37, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandCommaKeyDownPassThrough", keyCode: 43, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandSlashKeyDownPassThrough", keyCode: 44, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandShiftSlashKeyDownPassThrough", keyCode: 44, modifiers: [.command, .shift]),
    CommonCommandShortcut(name: "appleCommandTabKeyDownPassThrough", keyCode: 48, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandSpaceKeyDownPassThrough", keyCode: 49, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandBacktickKeyDownPassThrough", keyCode: 50, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandLeftBracketKeyDownPassThrough", keyCode: 33, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandRightBracketKeyDownPassThrough", keyCode: 30, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandEqualKeyDownPassThrough", keyCode: 24, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandMinusKeyDownPassThrough", keyCode: 27, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandDeleteKeyDownPassThrough", keyCode: 51, modifiers: [.command]),
    CommonCommandShortcut(name: "appleCommandControlQKeyDownPassThrough", keyCode: 12, modifiers: [.command, .control]),
    CommonCommandShortcut(name: "appleCommandShift3KeyDownPassThrough", keyCode: 20, modifiers: [.command, .shift]),
    CommonCommandShortcut(name: "appleCommandShift4KeyDownPassThrough", keyCode: 21, modifiers: [.command, .shift]),
    CommonCommandShortcut(name: "appleCommandShift5KeyDownPassThrough", keyCode: 23, modifiers: [.command, .shift]),
    CommonCommandShortcut(name: "appleCommandOptionEscapeKeyDownPassThrough", keyCode: 53, modifiers: [.command, .option]),
  ]
  private static let representativeKeyCodes: [UInt16] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9,
    11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
    21, 22, 23, 24, 25, 26, 27, 28, 29, 30,
    31, 32, 33, 34, 35, 37, 38, 40, 43, 44,
    45, 46, 47, 48, 49, 50, 51, 53, 76, 116,
    121, 123, 124, 125, 126,
  ]
  private static let commandModifierVariants: [NSEvent.ModifierFlags] = [
    .command,
    [.command, .shift],
    [.command, .option],
    [.command, .control],
    [.command, .option, .shift],
    [.command, .control, .shift],
    [.command, .control, .option],
    [.command, .control, .option, .shift],
  ]

  static func main() {
    let commandShortcutPassThroughChecks = commonCommandShortcutNames.map { name in
      let modifiers: NSEvent.ModifierFlags
      switch name {
      case "commandShiftVPassThrough":
        modifiers = [.command, .shift]
      case "commandOptionVPassThrough":
        modifiers = [.command, .option]
      case "commandOptionShiftVPassThrough":
        modifiers = [.command, .option, .shift]
      case "commandControlVPassThrough":
        modifiers = [.command, .control]
      case "commandShiftZPassThrough":
        modifiers = [.command, .shift]
      case "commandControlQPassThrough":
        modifiers = [.command, .control]
      case "commandShift3PassThrough", "commandShift4PassThrough", "commandShift5PassThrough":
        modifiers = [.command, .shift]
      case "commandOptionEscapePassThrough":
        modifiers = [.command, .option]
      default:
        modifiers = [.command]
      }
      return (name, InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: modifiers))
    }
    let commonAppleCommandShortcutSetPassesThrough = commandShortcutPassThroughChecks.allSatisfy { $0.1 }
    let officialAppleCommandKeyDownChecks = officialAppleCommandKeyDownShortcuts.map { shortcut in
      (
        shortcut.name,
        InputiaShortcutClassifier.shouldPassThroughKeyDown(
          keyCode: shortcut.keyCode,
          modifiers: shortcut.modifiers
        )
      )
    }
    let officialAppleCommandKeyDownSetPassesThrough = officialAppleCommandKeyDownChecks.allSatisfy { $0.1 }
    let anyCommandModifiedKeyPassesThrough = representativeKeyCodes.allSatisfy { keyCode in
      commandModifierVariants.allSatisfy { modifiers in
        InputiaShortcutClassifier.shouldPassThroughKeyDown(keyCode: keyCode, modifiers: modifiers)
      }
    }
    let allCommandModifierVariantsPassThrough = commandModifierVariants.allSatisfy { modifiers in
      InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: modifiers)
    }

    let checks: [(String, Bool)] = [
      (
        "commonAppleCommandShortcutSetPassesThrough",
        commonAppleCommandShortcutSetPassesThrough
      ),
      (
        "officialAppleCommandKeyDownSetPassesThrough",
        officialAppleCommandKeyDownSetPassesThrough
      ),
      (
        "anyCommandModifiedKeyPassesThrough",
        anyCommandModifiedKeyPassesThrough
      ),
      (
        "allCommandModifierVariantsPassThrough",
        allCommandModifierVariantsPassThrough
      ),
      (
        "commandlessShortcutNotForcedThrough",
        !InputiaShortcutClassifier.shouldPassThroughCommandShortcut(modifiers: [.control, .shift])
      ),
      (
        "ctrlPeriodPunctuation",
        InputiaShortcutClassifier.isPunctuationToggle(
          keyCode: keyCodePeriod,
          charactersIgnoringModifiers: ".",
          modifiers: [.control]
        )
      ),
      (
        "ctrlShiftPeriodRejected",
        !InputiaShortcutClassifier.isPunctuationToggle(
          keyCode: keyCodePeriod,
          charactersIgnoringModifiers: ".",
          modifiers: [.control, .shift]
        )
      ),
      (
        "ctrlCommandPeriodRejected",
        !InputiaShortcutClassifier.isPunctuationToggle(
          keyCode: keyCodePeriod,
          charactersIgnoringModifiers: ".",
          modifiers: [.control, .command]
        )
      ),
      (
        "shiftSpaceCharacterWidth",
        InputiaShortcutClassifier.isCharacterWidthToggle(
          keyCode: keyCodeSpace,
          modifiers: [.shift]
        )
      ),
      (
        "ctrlShiftSpaceRejected",
        !InputiaShortcutClassifier.isCharacterWidthToggle(
          keyCode: keyCodeSpace,
          modifiers: [.shift, .control]
        )
      ),
      (
        "plainSpaceRejected",
        !InputiaShortcutClassifier.isCharacterWidthToggle(
          keyCode: keyCodeSpace,
          modifiers: []
        )
      ),
      (
        "ctrlShiftVClipboardRecall",
        InputiaShortcutClassifier.isClipboardRecall(
          charactersIgnoringModifiers: "v",
          modifiers: [.control, .shift]
        )
      ),
      (
        "ctrlShiftCommandVRejected",
        !InputiaShortcutClassifier.isClipboardRecall(
          charactersIgnoringModifiers: "v",
          modifiers: [.control, .shift, .command]
        )
      ),
      (
        "ctrlShiftSScriptToggle",
        InputiaShortcutClassifier.isScriptToggle(
          charactersIgnoringModifiers: "s",
          modifiers: [.control, .shift],
          shortcut: "control_shift_s"
        )
      ),
      (
        "ctrlShiftCommandSScriptToggleRejected",
        !InputiaShortcutClassifier.isScriptToggle(
          charactersIgnoringModifiers: "s",
          modifiers: [.control, .shift, .command],
          shortcut: "control_shift_s"
        )
      ),
      (
        "scriptToggleRejectedWhenDisabled",
        !InputiaShortcutClassifier.isScriptToggle(
          charactersIgnoringModifiers: "s",
          modifiers: [.control, .shift],
          shortcut: "none"
        )
      ),
      (
        "shiftInputModeArmsWhenConfigured",
        InputiaShortcutClassifier.shouldArmShiftInputModeToggle(
          shortcut: "shift",
          modifiers: [.shift]
        )
      ),
      (
        "shiftInputModeRejectedWhenDisabled",
        !InputiaShortcutClassifier.shouldArmShiftInputModeToggle(
          shortcut: "none",
          modifiers: [.shift]
        )
      ),
      (
        "shiftInputModeReleaseTogglesWhenArmed",
        InputiaShortcutClassifier.isShiftInputModeToggleRelease(
          shortcut: "shift",
          hadShift: true,
          hasShift: false,
          hasBlockingModifier: false,
          armed: true
        )
      ),
      (
        "controlSpaceInputModeTogglesWhenConfigured",
        InputiaShortcutClassifier.isControlSpaceInputModeToggle(
          keyCode: keyCodeSpace,
          modifiers: [.control],
          shortcut: "control_space"
        )
      ),
      (
        "controlSpaceInputModeRejectedWhenShiftConfigured",
        !InputiaShortcutClassifier.isControlSpaceInputModeToggle(
          keyCode: keyCodeSpace,
          modifiers: [.control],
          shortcut: "shift"
        )
      ),
      (
        "rawCompositionOneSelectsFallback",
        InputiaShortcutClassifier.isDisplayedRawCompositionSelection(
          characters: "1",
          charactersIgnoringModifiers: "1",
          modifiers: [],
          hasComposing: true,
          hasCandidates: false
        )
      ),
      (
        "rawCompositionTwoRejected",
        !InputiaShortcutClassifier.isDisplayedRawCompositionSelection(
          characters: "2",
          charactersIgnoringModifiers: "2",
          modifiers: [],
          hasComposing: true,
          hasCandidates: false
        )
      ),
      (
        "rawCompositionOneRejectedWhenCandidatesExist",
        !InputiaShortcutClassifier.isDisplayedRawCompositionSelection(
          characters: "1",
          charactersIgnoringModifiers: "1",
          modifiers: [],
          hasComposing: true,
          hasCandidates: true
        )
      ),
      (
        "rawCompositionOneRejectedWithCommand",
        !InputiaShortcutClassifier.isDisplayedRawCompositionSelection(
          characters: "1",
          charactersIgnoringModifiers: "1",
          modifiers: [.command],
          hasComposing: true,
          hasCandidates: false
        )
      ),
      (
        "candidateDownArrowExpandsWhenComposing",
        InputiaShortcutClassifier.candidateNavigation(
          keyCode: keyCodeDownArrow,
          modifiers: [],
          hasComposing: true
        ) == .expandOrNextPage
      ),
      (
        "candidateUpArrowPagesWhenComposing",
        InputiaShortcutClassifier.candidateNavigation(
          keyCode: keyCodeUpArrow,
          modifiers: [],
          hasComposing: true
        ) == .previousPage
      ),
      (
        "candidateDownArrowRejectedWithoutComposition",
        InputiaShortcutClassifier.candidateNavigation(
          keyCode: keyCodeDownArrow,
          modifiers: [],
          hasComposing: false
        ) == nil
      ),
      (
        "candidateDownArrowRejectedWithCommand",
        InputiaShortcutClassifier.candidateNavigation(
          keyCode: keyCodeDownArrow,
          modifiers: [.command],
          hasComposing: true
        ) == nil
      ),
      (
        "inputTextCarriageReturnIsEnter",
        InputiaShortcutClassifier.isInputTextEnter("\r")
      ),
      (
        "inputTextLineFeedIsEnter",
        InputiaShortcutClassifier.isInputTextEnter("\n")
      ),
      (
        "inputTextLetterIsNotEnter",
        !InputiaShortcutClassifier.isInputTextEnter("n")
      ),
      (
        "inputTextSpaceHandledWhenComposing",
        InputiaShortcutClassifier.shouldHandleInputTextSpace(" ", hasComposing: true)
      ),
      (
        "inputTextSpacePassesThroughWithoutComposing",
        !InputiaShortcutClassifier.shouldHandleInputTextSpace(" ", hasComposing: false)
      ),
    ] + commandShortcutPassThroughChecks + officialAppleCommandKeyDownChecks

    let ok = checks.allSatisfy { $0.1 }
    print("shortcutSelfCheck=\(ok)")
    for (name, result) in checks {
      print("\(name)=\(result)")
    }
    exit(ok ? 0 : 1)
  }
}
