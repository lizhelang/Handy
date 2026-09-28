import AppKit
import Darwin

@main
struct InputiaShortcutSelfCheck {
  private static let keyCodeSpace: UInt16 = 49
  private static let keyCodePeriod: UInt16 = 47
  private static let keyCodeDownArrow: UInt16 = 125
  private static let keyCodeUpArrow: UInt16 = 126

  static func main() {
    func shiftedPunctuationDoesNotToggle(shiftKey: UInt16, shiftReleasedFirst: Bool, missingRelease: Bool) -> Bool {
      var gesture = InputiaShortcutClassifier.ShiftInputModeGestureState()
      gesture.observeLocalKeyDown(keyCode: shiftKey, modifiers: [.shift])
      _ = gesture.observeInputMethodFlagsChanged(shortcut: "shift", modifiers: [.shift])
      let deferred = InputiaShortcutClassifier.shouldConsumeDeferredShiftToggle(armed: gesture.isArmedForDebug, modifiers: [.shift])
      gesture.observeLocalKeyDown(keyCode: 44, modifiers: [.shift]) // Shift+/ -> ?
      guard !deferred else { return false }
      if missingRelease {
        gesture.observeLocalKeyUp(keyCode: 44)
        return !InputiaShortcutClassifier.shouldConsumeDeferredShiftToggle(armed: gesture.isArmedForDebug, modifiers: [])
      }
      if !shiftReleasedFirst { gesture.observeLocalKeyUp(keyCode: 44) }
      guard gesture.observeInputMethodFlagsChanged(shortcut: "shift", modifiers: []) == .none,
        !gesture.observePhysicalShiftKeyUp(shortcut: "shift", modifiers: []) else { return false }
      if shiftReleasedFirst { gesture.observeLocalKeyUp(keyCode: 44) }
      return !InputiaShortcutClassifier.shouldConsumeDeferredShiftToggle(armed: gesture.isArmedForDebug, modifiers: [])
    }
    let prefixChecks: [(String, Bool)] = [
      ("deferredShiftHeldQuestionRejected", !InputiaShortcutClassifier.shouldConsumeDeferredShiftToggle(armed: true, modifiers: [.shift])),
      ("deferredShiftUppercaseRejected", !InputiaShortcutClassifier.shouldConsumeDeferredShiftToggle(armed: true, modifiers: [.shift])),
      ("deferredShiftCommandRejected", !InputiaShortcutClassifier.shouldConsumeDeferredShiftToggle(armed: true, modifiers: [.command])),
      ("deferredShiftControlRejected", !InputiaShortcutClassifier.shouldConsumeDeferredShiftToggle(armed: true, modifiers: [.control])),
      ("deferredShiftOptionRejected", !InputiaShortcutClassifier.shouldConsumeDeferredShiftToggle(armed: true, modifiers: [.option])),
      ("deferredShiftPlainKeyRecoversMissingRelease", InputiaShortcutClassifier.shouldConsumeDeferredShiftToggle(armed: true, modifiers: [])),
      ("deferredShiftUnarmedRejected", !InputiaShortcutClassifier.shouldConsumeDeferredShiftToggle(armed: false, modifiers: [])),
    ] + [UInt16(56), 60].flatMap { shiftKey in
      [
        ("shiftQuestionKeyFirstNoToggle_\(shiftKey)", shiftedPunctuationDoesNotToggle(shiftKey: shiftKey, shiftReleasedFirst: false, missingRelease: false)),
        ("shiftQuestionShiftFirstNoToggle_\(shiftKey)", shiftedPunctuationDoesNotToggle(shiftKey: shiftKey, shiftReleasedFirst: true, missingRelease: false)),
        ("shiftQuestionMissingReleaseNoDeferredToggle_\(shiftKey)", shiftedPunctuationDoesNotToggle(shiftKey: shiftKey, shiftReleasedFirst: false, missingRelease: true)),
      ]
    }
    let shiftGestureChecks = InputiaShortcutClassifier.shiftInputModeGestureSelfCheckResults()
    let checks: [(String, Bool)] = [
      ("clipboardMenuDefault", InputiaShortcutClassifier.clipboardHistoryMenuTitle(shortcut: nil, enabled: nil) == "剪贴历史…（Ctrl + Shift + V）"),
      ("clipboardMenuCustom", InputiaShortcutClassifier.clipboardHistoryMenuTitle(shortcut: "ctrl+option+b", enabled: true) == "剪贴历史…（Ctrl + Option + B）"),
      ("clipboardMenuSymbolKey", InputiaShortcutClassifier.clipboardHistoryMenuTitle(shortcut: "ctrl+shift+/", enabled: true) == "剪贴历史…（Ctrl + Shift + /）"),
      ("clipboardMenuCmdOrCtrl", InputiaShortcutClassifier.clipboardHistoryMenuTitle(shortcut: "CmdOrCtrl+Shift+B", enabled: true) == "剪贴历史…（Cmd + Shift + B）"),
      ("clipboardMenuDisabled", InputiaShortcutClassifier.clipboardHistoryMenuTitle(shortcut: "ctrl+shift+v", enabled: false) == "剪贴历史…（Ctrl + Shift + V · 已停用）"),
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
        "shiftEnglishCompositionAcceptsUppercaseLetter",
        InputiaShortcutClassifier.isShiftEnglishCompositionCharacter(
          characters: "A",
          charactersIgnoringModifiers: "a",
          modifiers: [.shift]
        )
      ),
      (
        "shiftEnglishCompositionRejectsControlLetter",
        !InputiaShortcutClassifier.isShiftEnglishCompositionCharacter(
          characters: "A",
          charactersIgnoringModifiers: "a",
          modifiers: [.shift, .control]
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
        "expandedGridHasFiveRowsForFortyCandidates",
        InputiaExpandedCandidateGridNavigation.rowCount(candidateCount: 40, columnCount: 8) == 5
      ),
      (
        "expandedGridDownMovesToNextRowBeforePaging",
        InputiaExpandedCandidateGridNavigation.nextRow(
          currentRow: 0,
          candidateCount: 40,
          columnCount: 8
        ) == 1
      ),
      (
        "expandedGridDownStopsAtLastRow",
        InputiaExpandedCandidateGridNavigation.nextRow(
          currentRow: 4,
          candidateCount: 40,
          columnCount: 8
        ) == nil
      ),
      (
        "expandedGridUpMovesToPreviousRow",
        InputiaExpandedCandidateGridNavigation.previousRow(currentRow: 3) == 2
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
    ] + shiftGestureChecks + prefixChecks

    let ok = checks.allSatisfy { $0.1 }
    print("shortcutSelfCheck=\(ok)")
    for (name, result) in checks {
      print("\(name)=\(result)")
    }
    exit(ok ? 0 : 1)
  }
}
