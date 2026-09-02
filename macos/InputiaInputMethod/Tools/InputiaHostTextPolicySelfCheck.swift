import Darwin
import Foundation

@main
struct InputiaHostTextPolicySelfCheck {
  static func main() {
    let range = InputiaHostTextPolicy.replacementRange
    let appCommandPassThroughChecks = [
      "copy:",
      "paste:",
      "cut:",
      "undo:",
      "redo:",
      "selectAll:",
      "saveDocument:",
      "openDocument:",
      "performClose:",
      "terminate:",
      "find:",
      "orderFrontFindPanel:",
      "print:",
      "hide:",
      "showHelp:",
      "pasteAsPlainText:",
      "toggleContinuousSpellChecking:",
      "findNext:",
      "findPrevious:",
      "showPreferences:",
      "newDocument:",
      "saveDocumentAs:",
      "saveDocumentTo:",
      "duplicateDocument:",
      "miniaturize:",
      "performMiniaturize:",
      "performZoom:",
      "toggleFullScreen:",
      "toggleToolbarShown:",
      "toggleSidebar:",
      "toggleBold:",
      "toggleItalic:",
      "toggleUnderline:",
      "goBack:",
      "goForward:",
      "reload:",
      "stopLoading:",
    ].map { selectorName in
      (
        "appCommand\(selectorName.dropLast())PassesThrough",
        InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName: selectorName)
      )
    }
    let checks: [(String, Bool)] = [
      ("replacementRangeLocationIsNSNotFound", range.location == NSNotFound),
      ("replacementRangeLengthIsNSNotFound", range.length == NSNotFound),
      (
        "recallClipboardMenuHasNoCommandKeyEquivalent",
        InputiaHostTextPolicy.recallClipboardMenuKeyEquivalent.isEmpty
      ),
      (
        "settingsMenuHasNoCommandKeyEquivalent",
        InputiaHostTextPolicy.settingsMenuKeyEquivalent.isEmpty
      ),
      (
        "rawComposingFallbackCandidate",
        InputiaHostTextPolicy.candidatesForPanel(composing: "ni", candidates: []) == ["ni"]
      ),
      (
        "engineCandidatesPreferred",
        InputiaHostTextPolicy.candidatesForPanel(composing: "ni", candidates: ["你", "拟"]) == ["你", "拟"]
      ),
      (
        "emptyCompositionHasNoFallback",
        InputiaHostTextPolicy.candidatesForPanel(composing: "", candidates: []).isEmpty
      ),
      (
        "rawFallbackCanBeSelected",
        InputiaHostTextPolicy.isRawFallbackSelection(
          selected: "abc",
          composing: "abc",
          candidates: []
        )
      ),
      (
        "rawFallbackRejectedWhenEngineCandidatesExist",
        !InputiaHostTextPolicy.isRawFallbackSelection(
          selected: "abc",
          composing: "abc",
          candidates: ["阿"]
        )
      ),
      (
        "clearMarkedTextWhenCompositionClearsWithoutCommit",
        InputiaHostTextPolicy.shouldClearMarkedText(
          previousComposing: "ni",
          nextComposing: "",
          committedText: nil
        )
      ),
      (
        "commitDoesNotAlsoClearMarkedText",
        !InputiaHostTextPolicy.shouldClearMarkedText(
          previousComposing: "ni",
          nextComposing: "",
          committedText: "ni"
        )
      ),
      (
        "partialCandidateCommitContinuesMarkedText",
        InputiaHostTextPolicy.shouldContinueMarkedTextAfterCommit(
          committedText: "你",
          nextComposing: "lllema"
        )
      ),
      (
        "fullCandidateCommitDoesNotContinueMarkedText",
        !InputiaHostTextPolicy.shouldContinueMarkedTextAfterCommit(
          committedText: "你来了吗",
          nextComposing: ""
        )
      ),
      (
        "emptyCommitDoesNotContinueMarkedText",
        !InputiaHostTextPolicy.shouldContinueMarkedTextAfterCommit(
          committedText: "",
          nextComposing: "lllema"
        )
      ),
      (
        "commitUsesMarkedRangeWhenComposing",
        InputiaHostTextPolicy.commitReplacementRange(
          previousComposing: "ni",
          markedRange: NSRange(location: 7, length: 2)
        ) == NSRange(location: 7, length: 2)
      ),
      (
        "commitFallsBackWithoutComposition",
        InputiaHostTextPolicy.commitReplacementRange(
          previousComposing: "",
          markedRange: NSRange(location: 7, length: 2)
        ) == range
      ),
      (
        "commitFallsBackWhenMarkedRangeUnavailable",
        InputiaHostTextPolicy.commitReplacementRange(
          previousComposing: "ni",
          markedRange: NSRange(location: NSNotFound, length: 0)
        ) == range
      ),
      (
        "emptyNewlineCommandPassesThrough",
        InputiaHostTextPolicy.shouldPassThroughNewlineCommand(
          selectorName: "insertNewline:",
          hasComposing: false
        )
      ),
      (
        "composingNewlineCommandIsHandled",
        !InputiaHostTextPolicy.shouldPassThroughNewlineCommand(
          selectorName: "insertNewline:",
          hasComposing: true
        )
      ),
      (
        "emptyLineBreakCommandPassesThrough",
        InputiaHostTextPolicy.shouldPassThroughNewlineCommand(
          selectorName: "insertLineBreak:",
          hasComposing: false
        )
      ),
      (
        "composingLineBreakCommandIsHandled",
        !InputiaHostTextPolicy.shouldPassThroughNewlineCommand(
          selectorName: "insertLineBreak:",
          hasComposing: true
        )
      ),
      (
        "deleteBackwardIsNotAppCommand",
        !InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName: "deleteBackward:")
      ),
      (
        "moveDownIsNotAppCommand",
        !InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName: "moveDown:")
      ),
      (
        "insertTabIsNotAppCommand",
        !InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName: "insertTab:")
      ),
      (
        "showPreferencesCommandPassesThrough",
        InputiaHostTextPolicy.shouldPassThroughAppCommand(selectorName: "showPreferences:")
      ),
      (
        "asciiRawFallbackNewlinePassesThroughAfterCommit",
        InputiaHostTextPolicy.shouldPassThroughNewlineAfterRawFallbackCommit(
          previousComposing: "abc",
          candidates: [],
          committedText: "abc"
        )
      ),
      (
        "candidateCommitNewlineDoesNotPassThrough",
        !InputiaHostTextPolicy.shouldPassThroughNewlineAfterRawFallbackCommit(
          previousComposing: "ni",
          candidates: ["你"],
          committedText: "你"
        )
      ),
      (
        "nonAsciiRawFallbackNewlineDoesNotPassThrough",
        !InputiaHostTextPolicy.shouldPassThroughNewlineAfterRawFallbackCommit(
          previousComposing: "你好",
          candidates: [],
          committedText: "你好"
        )
      ),
      (
        "changedCommitNewlineDoesNotPassThrough",
        !InputiaHostTextPolicy.shouldPassThroughNewlineAfterRawFallbackCommit(
          previousComposing: "abc",
          candidates: [],
          committedText: "ab"
        )
      ),
    ] + appCommandPassThroughChecks

    let ok = checks.allSatisfy { $0.1 }
    print("hostTextPolicySelfCheck=\(ok)")
    for (name, passed) in checks {
      print("\(name)=\(passed)")
    }
    exit(ok ? 0 : 1)
  }
}
