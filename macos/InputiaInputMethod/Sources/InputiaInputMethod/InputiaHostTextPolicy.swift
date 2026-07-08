import Foundation

enum InputiaHostTextPolicy {
  static let replacementRange = NSRange(location: NSNotFound, length: NSNotFound)
  static let recallClipboardMenuKeyEquivalent = ""
  private static let newlineCommandSelectors: Set<String> = [
    "insertNewline:",
    "insertLineBreak:",
  ]
  private static let appCommandSelectors: Set<String> = [
    "arrangeInFront:",
    "changeFont:",
    "capitalizeWord:",
    "centerSelectionInVisibleArea:",
    "close:",
    "copy:",
    "copyFont:",
    "copyRuler:",
    "cut:",
    "delete:",
    "duplicateDocument:",
    "enterFullScreenMode:",
    "findNext:",
    "findPrevious:",
    "find:",
    "goBack:",
    "goForward:",
    "hide:",
    "hideOtherApplications:",
    "lockDocument:",
    "makeTextWritingDirectionLeftToRight:",
    "makeTextWritingDirectionRightToLeft:",
    "miniaturize:",
    "moveDocument:",
    "newDocument:",
    "openDocument:",
    "orderFrontCharacterPalette:",
    "orderFrontFindPanel:",
    "orderFrontStandardAboutPanel:",
    "paste:",
    "pasteAsPlainText:",
    "pasteAsRichText:",
    "pasteFont:",
    "pasteRuler:",
    "performClose:",
    "performMiniaturize:",
    "performZoom:",
    "print:",
    "reload:",
    "renameDocument:",
    "redo:",
    "revertDocumentToSaved:",
    "runPageLayout:",
    "runToolbarCustomizationPalette:",
    "saveAllDocuments:",
    "saveDocument:",
    "saveDocumentAs:",
    "saveDocumentTo:",
    "selectAll:",
    "showHelp:",
    "showPreferences:",
    "showWindow:",
    "startSpeaking:",
    "stopLoading:",
    "stopSpeaking:",
    "terminate:",
    "toggleAutomaticDashSubstitution:",
    "toggleAutomaticDataDetection:",
    "toggleAutomaticLinkDetection:",
    "toggleAutomaticQuoteSubstitution:",
    "toggleAutomaticSpellingCorrection:",
    "toggleAutomaticTextCompletion:",
    "toggleBold:",
    "toggleContinuousSpellChecking:",
    "toggleFullScreen:",
    "toggleItalic:",
    "toggleSmartInsertDelete:",
    "toggleSidebar:",
    "toggleToolbarShown:",
    "toggleUnderline:",
    "transpose:",
    "transposeWords:",
    "undo:",
    "uppercaseWord:",
    "yank:",
  ]

  static func candidatesForPanel(composing: String, candidates: [String]) -> [String] {
    if candidates.isEmpty && !composing.isEmpty {
      return [composing]
    }
    return candidates
  }

  static func isRawFallbackSelection(
    selected: String,
    composing: String,
    candidates: [String]
  ) -> Bool {
    candidates.isEmpty && !composing.isEmpty && selected == composing
  }

  static func shouldClearMarkedText(
    previousComposing: String,
    nextComposing: String,
    committedText: String?
  ) -> Bool {
    !previousComposing.isEmpty
      && nextComposing.isEmpty
      && (committedText ?? "").isEmpty
  }

  static func commitReplacementRange(previousComposing: String, markedRange: NSRange) -> NSRange {
    guard !previousComposing.isEmpty else {
      return replacementRange
    }
    guard markedRange.location != NSNotFound else {
      return replacementRange
    }
    return markedRange
  }

  static func isNewlineCommand(selectorName: String) -> Bool {
    newlineCommandSelectors.contains(selectorName)
  }

  static func shouldPassThroughNewlineCommand(selectorName: String, hasComposing: Bool) -> Bool {
    isNewlineCommand(selectorName: selectorName) && !hasComposing
  }

  static func shouldPassThroughAppCommand(selectorName: String) -> Bool {
    appCommandSelectors.contains(selectorName)
  }

  static func shouldPassThroughNewlineAfterRawFallbackCommit(
    previousComposing: String,
    candidates: [String],
    committedText: String?
  ) -> Bool {
    guard !previousComposing.isEmpty, candidates.isEmpty else {
      return false
    }
    guard committedText == previousComposing else {
      return false
    }
    return previousComposing.unicodeScalars.allSatisfy { scalar in
      scalar.isASCII && scalar.value >= 0x20 && scalar.value != 0x7f
    }
  }
}
