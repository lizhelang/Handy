enum InputiaInputTextAction: Equatable {
  case passthrough
  case enter
  case space
  case character(Character)
}

struct InputiaInputTextRouter {
  static func action(for character: Character, hasComposing: Bool) -> InputiaInputTextAction {
    let text = String(character)
    if InputiaShortcutClassifier.isInputTextEnter(text) {
      return hasComposing ? .enter : .passthrough
    }
    if InputiaShortcutClassifier.shouldHandleInputTextSpace(text, hasComposing: hasComposing) {
      return .space
    }
    return .character(character)
  }
}
