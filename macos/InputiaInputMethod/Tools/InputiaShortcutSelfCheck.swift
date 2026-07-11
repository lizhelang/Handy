import AppKit
import Darwin

@main
struct InputiaShortcutSelfCheck {
  private static let keyCodeSpace: UInt16 = 49
  private static let keyCodePeriod: UInt16 = 47
  private static let keyCodeDownArrow: UInt16 = 125
  private static let keyCodeUpArrow: UInt16 = 126

  static func main() {
    let checks: [(String, Bool)] = [
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
    ]

    let ok = checks.allSatisfy { $0.1 }
    print("shortcutSelfCheck=\(ok)")
    for (name, result) in checks {
      print("\(name)=\(result)")
    }
    exit(ok ? 0 : 1)
  }
}
