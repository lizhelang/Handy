import AppKit

enum InputiaCandidateNavigation: Equatable {
  case expandOrNextPage
  case previousPage
}

struct InputiaShortcutClassifier {
  private static let keyCodeSpace: UInt16 = 49
  private static let keyCodePeriod: UInt16 = 47
  private static let keyCodeDownArrow: UInt16 = 125
  private static let keyCodeUpArrow: UInt16 = 126
  private static let inputTextEnterCharacters: Set<String> = ["\r", "\n"]

  static func isClipboardRecall(
    charactersIgnoringModifiers: String?,
    modifiers: NSEvent.ModifierFlags
  ) -> Bool {
    guard modifiers.contains(.control), modifiers.contains(.shift) else {
      return false
    }
    guard !modifiers.contains(.command), !modifiers.contains(.option) else {
      return false
    }
    return charactersIgnoringModifiers?.lowercased() == "v"
  }

  static func isScriptToggle(
    charactersIgnoringModifiers: String?,
    modifiers: NSEvent.ModifierFlags,
    shortcut: String
  ) -> Bool {
    guard shortcut == "control_shift_s" else {
      return false
    }
    guard modifiers.contains(.control), modifiers.contains(.shift) else {
      return false
    }
    guard !modifiers.contains(.command), !modifiers.contains(.option) else {
      return false
    }
    return charactersIgnoringModifiers?.lowercased() == "s"
  }

  static func isPunctuationToggle(
    keyCode: UInt16,
    charactersIgnoringModifiers: String?,
    modifiers: NSEvent.ModifierFlags
  ) -> Bool {
    guard modifiers.contains(.control), !modifiers.contains(.shift) else {
      return false
    }
    guard !modifiers.contains(.command), !modifiers.contains(.option) else {
      return false
    }
    return keyCode == keyCodePeriod || charactersIgnoringModifiers == "."
  }

  static func isCharacterWidthToggle(
    keyCode: UInt16,
    modifiers: NSEvent.ModifierFlags
  ) -> Bool {
    guard modifiers.contains(.shift) else {
      return false
    }
    guard !modifiers.contains(.command), !modifiers.contains(.control), !modifiers.contains(.option) else {
      return false
    }
    return keyCode == keyCodeSpace
  }

  static func shouldArmShiftInputModeToggle(
    shortcut: String,
    modifiers: NSEvent.ModifierFlags
  ) -> Bool {
    guard shortcut == "shift" else {
      return false
    }
    return modifiers.contains(.shift)
      && !modifiers.contains(.command)
      && !modifiers.contains(.control)
      && !modifiers.contains(.option)
  }

  static func isShiftInputModeToggleRelease(
    shortcut: String,
    hadShift: Bool,
    hasShift: Bool,
    hasBlockingModifier: Bool,
    armed: Bool
  ) -> Bool {
    shortcut == "shift" && hadShift && !hasShift && armed && !hasBlockingModifier
  }

  static func isControlSpaceInputModeToggle(
    keyCode: UInt16,
    modifiers: NSEvent.ModifierFlags,
    shortcut: String
  ) -> Bool {
    guard shortcut == "control_space" else {
      return false
    }
    guard modifiers.contains(.control) else {
      return false
    }
    guard !modifiers.contains(.command), !modifiers.contains(.option), !modifiers.contains(.shift) else {
      return false
    }
    return keyCode == keyCodeSpace
  }

  static func isDisplayedRawCompositionSelection(
    characters: String?,
    charactersIgnoringModifiers: String?,
    modifiers: NSEvent.ModifierFlags,
    hasComposing: Bool,
    hasCandidates: Bool
  ) -> Bool {
    guard hasComposing, !hasCandidates else {
      return false
    }
    guard !modifiers.contains(.command), !modifiers.contains(.control), !modifiers.contains(.option) else {
      return false
    }
    return characters == "1" && charactersIgnoringModifiers == "1"
  }

  static func candidateNavigation(
    keyCode: UInt16,
    modifiers: NSEvent.ModifierFlags,
    hasComposing: Bool
  ) -> InputiaCandidateNavigation? {
    guard hasComposing else {
      return nil
    }
    guard !modifiers.contains(.command),
      !modifiers.contains(.control),
      !modifiers.contains(.option),
      !modifiers.contains(.shift)
    else {
      return nil
    }

    switch keyCode {
    case keyCodeDownArrow:
      return .expandOrNextPage
    case keyCodeUpArrow:
      return .previousPage
    default:
      return nil
    }
  }

  static func isInputTextEnter(_ text: String) -> Bool {
    inputTextEnterCharacters.contains(text)
  }

  static func shouldHandleInputTextSpace(_ text: String, hasComposing: Bool) -> Bool {
    text == " " && hasComposing
  }
}
