import Foundation

struct InputiaSecureDirectDecision: Equatable {
  let passthrough: Bool
  let clearComposition: Bool
  let hideCandidates: Bool
  let blockClipboardRecall: Bool
  let readsContext: Bool
  let callsBridgeHandle: Bool
  let setsMarkedTextForInput: Bool
  let showsCandidates: Bool
  let learns: Bool
}

enum InputiaHostTextPolicy {
  static let replacementRange = NSRange(location: NSNotFound, length: NSNotFound)
  static let recallClipboardMenuKeyEquivalent = ""
  static let settingsMenuKeyEquivalent = ""
  private static let secureDirectBundleIdentifiers: Set<String> = [
    "com.apple.SecurityAgent",
  ]
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

  static func shouldContinueCompositionAfterCommit(
    nextComposing: String,
    committedText: String?
  ) -> Bool {
    !(committedText ?? "").isEmpty && !nextComposing.isEmpty
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

  static func isSecureDirectBundleIdentifier(_ bundleIdentifier: String?) -> Bool {
    guard let bundleIdentifier else {
      return false
    }
    return secureDirectBundleIdentifiers.contains(
      bundleIdentifier.trimmingCharacters(in: .whitespacesAndNewlines)
    )
  }

  static func shouldPassThroughSecureDirectText(
    _ text: String,
    bundleIdentifier: String?
  ) -> Bool {
    isSecureDirectBundleIdentifier(bundleIdentifier) && !text.isEmpty
  }

  static func secureDirectDecision(bundleIdentifier: String?) -> InputiaSecureDirectDecision {
    let passthrough = isSecureDirectBundleIdentifier(bundleIdentifier)
    return InputiaSecureDirectDecision(
      passthrough: passthrough,
      clearComposition: passthrough,
      hideCandidates: passthrough,
      blockClipboardRecall: passthrough,
      readsContext: false,
      callsBridgeHandle: false,
      setsMarkedTextForInput: false,
      showsCandidates: false,
      learns: false
    )
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
