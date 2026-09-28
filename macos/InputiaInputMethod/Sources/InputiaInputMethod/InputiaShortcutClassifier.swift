import AppKit

enum InputiaCandidateNavigation: Equatable {
  case expandOrNextPage
  case previousPage
}

struct InputiaShortcutClassifier {
  enum ShiftInputModeGestureResult: Equatable {
    case none
    case toggle
  }

  struct ShiftInputModeGestureState {
    private static let modifierKeyCodes: Set<UInt16> = [54, 55, 56, 57, 58, 59, 60, 61, 62, 63]

    private var lastModifiers = NSEvent.ModifierFlags()
    private var activeNonModifierKeyCodes = Set<UInt16>()
    private var armed = false
    private var invalidated = false

    var isArmedForDebug: Bool {
      armed && !invalidated
    }

    var hasHeldKeysForDebug: Bool { !activeNonModifierKeyCodes.isEmpty }
    var hasShiftBaselineForDebug: Bool { lastModifiers.contains(.shift) }

    mutating func reconcileHeldKeys(isKeyDown: (UInt16) -> Bool) {
      activeNonModifierKeyCodes = Set(activeNonModifierKeyCodes.filter(isKeyDown))
    }

    mutating func cancelPendingGesture() {
      // 取消资格不等于物理松开。保留状态，避免另一路同一按下事件重新武装。
      armed = false
      invalidated = true
    }

    mutating func resetSession(modifiers: NSEvent.ModifierFlags) {
      activeNonModifierKeyCodes.removeAll(keepingCapacity: true)
      lastModifiers = modifiers
      armed = false
      invalidated = true
    }

    mutating func observeGlobalKeyDown(keyCode: UInt16, modifiers: NSEvent.ModifierFlags) {
      guard !Self.modifierKeyCodes.contains(keyCode) else {
        return
      }
      activeNonModifierKeyCodes.insert(keyCode)
      if modifiers.contains(.shift) || lastModifiers.contains(.shift) {
        invalidated = true
      }
    }

    mutating func observeGlobalKeyUp(keyCode: UInt16) {
      activeNonModifierKeyCodes.remove(keyCode)
    }

    mutating func observeLocalKeyDown(keyCode: UInt16, modifiers: NSEvent.ModifierFlags) {
      observeGlobalKeyDown(keyCode: keyCode, modifiers: modifiers)
    }

    mutating func observeLocalKeyUp(keyCode: UInt16) {
      observeGlobalKeyUp(keyCode: keyCode)
    }

    mutating func observePhysicalShiftKeyUp(
      shortcut: String,
      modifiers: NSEvent.ModifierFlags
    ) -> Bool {
      let shouldToggle = shortcut == "shift"
        && armed
        && !invalidated
        && activeNonModifierKeyCodes.isEmpty
        && !modifiers.contains(.command)
        && !modifiers.contains(.control)
        && !modifiers.contains(.option)
      armed = false
      invalidated = false
      lastModifiers = modifiers
      return shouldToggle
    }

    mutating func observeInputMethodFlagsChanged(
      shortcut: String,
      modifiers: NSEvent.ModifierFlags
    ) -> ShiftInputModeGestureResult {
      // IMK 可能只交付普通键 keyDown，不能用历史 keyDown 推断当前仍按住。
      // 当前 Shift 手势期间的新按键仍会取消资格，保留大写输入和组合键行为。
      if !lastModifiers.contains(.shift) && modifiers.contains(.shift) {
        activeNonModifierKeyCodes.removeAll(keepingCapacity: true)
      }
      return observeFlagsChanged(shortcut: shortcut, modifiers: modifiers, allowToggle: true)
    }

    mutating func observeFlagsChanged(
      shortcut: String,
      modifiers: NSEvent.ModifierFlags,
      allowToggle: Bool
    ) -> ShiftInputModeGestureResult {
      let hadShift = lastModifiers.contains(.shift)
      let hasShift = modifiers.contains(.shift)
      let hasBlockingModifier = modifiers.contains(.command)
        || modifiers.contains(.control)
        || modifiers.contains(.option)

      // 全局监听只否决组合参与；只有带 IMK client 的本地事件拥有手势起止。
      // 不保存待领取输出，避免迟到的全局事件制造第二次切换。
      if !allowToggle {
        if hadShift && hasBlockingModifier {
          invalidated = true
        }
        return .none
      }

      if !hadShift && hasShift {
        armed = InputiaShortcutClassifier.shouldArmShiftInputModeToggle(
          shortcut: shortcut,
          modifiers: modifiers
        ) && activeNonModifierKeyCodes.isEmpty
        invalidated = hasBlockingModifier || !activeNonModifierKeyCodes.isEmpty
      } else if hadShift && hasShift {
        if hasBlockingModifier {
          invalidated = true
        }
      } else if hadShift && !hasShift {
        let shouldToggle = InputiaShortcutClassifier.isShiftInputModeToggleRelease(
          shortcut: shortcut,
          hadShift: hadShift,
          hasShift: hasShift,
          hasBlockingModifier: hasBlockingModifier,
          armed: armed && !invalidated
        )
        armed = false
        invalidated = false
        // Shift 松开不意味着普通键也已松开；由其 keyUp 清除按住状态。
        lastModifiers = modifiers
        return shouldToggle ? .toggle : .none
      }

      lastModifiers = modifiers
      return .none
    }
  }

  static func shiftInputModeGestureSelfCheckResults() -> [(String, Bool)] {
    var imeMissingKeyUp = ShiftInputModeGestureState()
    imeMissingKeyUp.observeLocalKeyDown(keyCode: 0, modifiers: [])
    _ = imeMissingKeyUp.observeInputMethodFlagsChanged(shortcut: "shift", modifiers: [.shift])
    let imeFirstToggle = imeMissingKeyUp.observeInputMethodFlagsChanged(shortcut: "shift", modifiers: []) == .toggle
    imeMissingKeyUp.observeLocalKeyDown(keyCode: 11, modifiers: [])
    _ = imeMissingKeyUp.observeInputMethodFlagsChanged(shortcut: "shift", modifiers: [.shift])
    let imeReturnToggle = imeMissingKeyUp.observeInputMethodFlagsChanged(shortcut: "shift", modifiers: []) == .toggle
    _ = imeMissingKeyUp.observeInputMethodFlagsChanged(shortcut: "shift", modifiers: [.shift])
    imeMissingKeyUp.observeLocalKeyDown(keyCode: 0, modifiers: [.shift])
    let imeUppercaseRejectsToggle = imeMissingKeyUp.observeInputMethodFlagsChanged(shortcut: "shift", modifiers: []) == .none
    let keyCodeV: UInt16 = 9
    var independent = ShiftInputModeGestureState()
    let independentDown = independent.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [.shift],
      allowToggle: true
    ) == .none
    let independentUp = independent.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [],
      allowToggle: true
    ) == .toggle
    let independentSecondDown = independent.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [.shift],
      allowToggle: true
    ) == .none
    let independentSecondUp = independent.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [],
      allowToggle: true
    ) == .toggle

    var physicalRelease = ShiftInputModeGestureState()
    _ = physicalRelease.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [.shift],
      allowToggle: true
    )
    let physicalReleaseToggles = physicalRelease.observePhysicalShiftKeyUp(
      shortcut: "shift",
      modifiers: []
    )

    var modifierReleasedBeforeShift = ShiftInputModeGestureState()
    _ = modifierReleasedBeforeShift.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [.shift],
      allowToggle: true
    )
    let modifierJoin = modifierReleasedBeforeShift.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [.shift, .control],
      allowToggle: true
    ) == .none
    let modifierLeavesFirst = modifierReleasedBeforeShift.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [.shift],
      allowToggle: true
    ) == .none
    let modifierReleaseRejected = modifierReleasedBeforeShift.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [],
      allowToggle: true
    ) == .none

    var keyDuringShift = ShiftInputModeGestureState()
    _ = keyDuringShift.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    keyDuringShift.observeLocalKeyDown(keyCode: keyCodeV, modifiers: [.shift])
    let keyDuringShiftRejected = keyDuringShift.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [],
      allowToggle: true
    ) == .none

    var keyBeforeShift = ShiftInputModeGestureState()
    keyBeforeShift.observeGlobalKeyDown(keyCode: keyCodeV, modifiers: [])
    let keyBeforeShiftDown = keyBeforeShift.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [.shift],
      allowToggle: true
    ) == .none
    keyBeforeShift.observeGlobalKeyUp(keyCode: keyCodeV)
    let keyBeforeShiftRejected = keyBeforeShift.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [],
      allowToggle: true
    ) == .none

    var overlappingShift = ShiftInputModeGestureState()
    let leftDown = overlappingShift.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [.shift],
      allowToggle: true
    ) == .none
    let rightDown = overlappingShift.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [.shift],
      allowToggle: true
    ) == .none
    let leftUp = overlappingShift.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [.shift],
      allowToggle: true
    ) == .none
    let rightUp = overlappingShift.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [],
      allowToggle: true
    ) == .toggle

    var globalFirst = ShiftInputModeGestureState()
    _ = globalFirst.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    let globalReleaseDoesNotToggle = globalFirst.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [],
      allowToggle: false
    ) == .none
    let localReleaseOwnsToggle = globalFirst.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [],
      allowToggle: true
    ) == .toggle
    let duplicateLocalReleaseRejected = globalFirst.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [],
      allowToggle: true
    ) == .none

    var focusCancelled = ShiftInputModeGestureState()
    _ = focusCancelled.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    focusCancelled.cancelPendingGesture()
    let focusCancelRejectsRelease = focusCancelled.observeFlagsChanged(
      shortcut: "shift",
      modifiers: [],
      allowToggle: true
    ) == .none

    var disabled = ShiftInputModeGestureState()
    _ = disabled.observeFlagsChanged(shortcut: "none", modifiers: [.shift], allowToggle: true)
    let disabledReleaseRejected = disabled.observeFlagsChanged(
      shortcut: "none",
      modifiers: [],
      allowToggle: true
    ) == .none

    var heldAcrossGestures = ShiftInputModeGestureState()
    heldAcrossGestures.observeGlobalKeyDown(keyCode: keyCodeV, modifiers: [])
    _ = heldAcrossGestures.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    _ = heldAcrossGestures.observeFlagsChanged(shortcut: "shift", modifiers: [], allowToggle: true)
    _ = heldAcrossGestures.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    let heldAcrossGesturesRejected = heldAcrossGestures.observeFlagsChanged(
      shortcut: "shift", modifiers: [], allowToggle: true
    ) == .none

    var interveningKey = ShiftInputModeGestureState()
    _ = interveningKey.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    _ = interveningKey.observeFlagsChanged(shortcut: "shift", modifiers: [], allowToggle: false)
    interveningKey.observeLocalKeyDown(keyCode: keyCodeV, modifiers: [])
    let interveningKeyRejected = interveningKey.observeFlagsChanged(
      shortcut: "shift", modifiers: [], allowToggle: true
    ) == .none

    var localHeldKey = ShiftInputModeGestureState()
    localHeldKey.observeLocalKeyDown(keyCode: keyCodeV, modifiers: [])
    _ = localHeldKey.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    let localHeldKeyRejected = localHeldKey.observeFlagsChanged(
      shortcut: "shift", modifiers: [], allowToggle: true
    ) == .none
    localHeldKey.observeLocalKeyUp(keyCode: keyCodeV)
    _ = localHeldKey.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    let localReleaseRestoresIndependentShift = localHeldKey.observeFlagsChanged(
      shortcut: "shift", modifiers: [], allowToggle: true
    ) == .toggle

    var cancelledHold = ShiftInputModeGestureState()
    _ = cancelledHold.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    cancelledHold.cancelPendingGesture()
    _ = cancelledHold.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: false)
    let cancelledHoldRejectsDuplicateDown = cancelledHold.observeFlagsChanged(
      shortcut: "shift", modifiers: [], allowToggle: true
    ) == .none

    var delayedGlobal = ShiftInputModeGestureState()
    _ = delayedGlobal.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    _ = delayedGlobal.observeFlagsChanged(shortcut: "shift", modifiers: [], allowToggle: true)
    _ = delayedGlobal.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: false)
    _ = delayedGlobal.observeFlagsChanged(shortcut: "shift", modifiers: [], allowToggle: false)
    let delayedGlobalCannotCreateSecondToggle = delayedGlobal.observeFlagsChanged(
      shortcut: "shift", modifiers: [], allowToggle: true
    ) == .none

    var changedSession = ShiftInputModeGestureState()
    changedSession.observeLocalKeyDown(keyCode: keyCodeV, modifiers: [])
    changedSession.resetSession(modifiers: [])
    _ = changedSession.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    let newSessionRecoversFromMissingKeyUp = changedSession.observeFlagsChanged(
      shortcut: "shift", modifiers: [], allowToggle: true
    ) == .toggle
    changedSession.resetSession(modifiers: [.shift])
    _ = changedSession.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    let newSessionRejectsAlreadyHeldShift = changedSession.observeFlagsChanged(
      shortcut: "shift", modifiers: [], allowToggle: true
    ) == .none

    var missingRelease = ShiftInputModeGestureState()
    missingRelease.observeLocalKeyDown(keyCode: keyCodeV, modifiers: [])
    missingRelease.reconcileHeldKeys { _ in false }
    _ = missingRelease.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    let physicalReleaseRecovers = missingRelease.observeFlagsChanged(shortcut: "shift", modifiers: [], allowToggle: true) == .toggle
    var realHold = ShiftInputModeGestureState()
    realHold.observeLocalKeyDown(keyCode: keyCodeV, modifiers: [])
    realHold.reconcileHeldKeys { _ in true }
    _ = realHold.observeFlagsChanged(shortcut: "shift", modifiers: [.shift], allowToggle: true)
    realHold.observeLocalKeyUp(keyCode: keyCodeV)
    let physicalHoldStillRejects = realHold.observeFlagsChanged(shortcut: "shift", modifiers: [], allowToggle: true) == .none

    return [
      ("shiftGestureIMKMissingKeyUpAllowsBothDirections", imeFirstToggle && imeReturnToggle),
      ("shiftGestureIMKUppercaseDoesNotToggle", imeUppercaseRejectsToggle),
      ("shiftGesturePhysicalReleaseRecoversMissingKeyUp", physicalReleaseRecovers),
      ("shiftGesturePhysicalHoldStillRejectsAfterEarlierKeyUp", physicalHoldStillRejects),
      ("shiftGestureNewSessionRecoversMissingKeyUp", newSessionRecoversFromMissingKeyUp),
      ("shiftGestureNewSessionRejectsAlreadyHeldShift", newSessionRejectsAlreadyHeldShift),
      ("shiftGestureDelayedGlobalCannotCreateSecondToggle", delayedGlobalCannotCreateSecondToggle),
      ("shiftGestureCancelledHoldCannotRearmFromDuplicateDown", cancelledHoldRejectsDuplicateDown),
      ("shiftGestureLocalHeldKeyRejectedWithoutGlobalMonitor", localHeldKeyRejected),
      ("shiftGestureLocalKeyUpRestoresIndependentShift", localReleaseRestoresIndependentShift),
      ("shiftGestureHeldKeyAcrossTwoGesturesRejected", heldAcrossGesturesRejected),
      ("shiftGestureInterveningKeyInvalidatesLocalRelease", interveningKeyRejected),
      ("shiftGestureIndependentFirstToggles", independentDown && independentUp),
      ("shiftGestureIndependentSecondTogglesWithoutTimeWindow", independentSecondDown && independentSecondUp),
      ("shiftGesturePhysicalKeyUpToggles", physicalReleaseToggles),
      (
        "shiftGestureRejectsModifierReleasedBeforeShift",
        modifierJoin && modifierLeavesFirst && modifierReleaseRejected
      ),
      ("shiftGestureRejectsKeyDuringShift", keyDuringShiftRejected),
      ("shiftGestureRejectsKeyPressedBeforeShift", keyBeforeShiftDown && keyBeforeShiftRejected),
      (
        "shiftGestureOverlappingLeftRightShiftTogglesOnce",
        leftDown && rightDown && leftUp && rightUp
      ),
      (
        "shiftGestureGlobalReleaseLeavesLocalAsSoleOwner",
        globalReleaseDoesNotToggle && localReleaseOwnsToggle && duplicateLocalReleaseRejected
      ),
      ("shiftGestureFocusCancelRejectsRelease", focusCancelRejectsRelease),
      ("shiftGestureDisabledShortcutRejectsRelease", disabledReleaseRejected),
    ]
  }

  static func isVoiceInput(keyCode: UInt16, characters: String?, modifiers: NSEvent.ModifierFlags) -> Bool {
    let voiceKey = keyCode == 9 || characters?.lowercased() == "v" || characters == "\u{16}"
    return voiceKey && modifiers.contains([.control, .option, .shift]) && !modifiers.contains(.command)
  }
  private static let keyCodeSpace: UInt16 = 49
  private static let keyCodePeriod: UInt16 = 47
  private static let keyCodeDownArrow: UInt16 = 125
  private static let keyCodeUpArrow: UInt16 = 126
  private static let inputTextEnterCharacters: Set<String> = ["\r", "\n"]

  /// 菜单显示实际设置的快捷键；兼容尚未返回该字段的旧服务。
  static func clipboardHistoryMenuTitle(shortcut: String?, enabled: Bool?) -> String {
    let raw = shortcut ?? "ctrl+shift+v"
    let labels = ["ctrl": "Ctrl", "control": "Ctrl", "shift": "Shift", "command": "Cmd",
      "super": "Cmd", "cmd": "Cmd", "cmdorctrl": "Cmd", "option": "Option", "alt": "Option", "space": "Space"]
    let parts = raw.split(separator: "+").prefix(5).map { part -> String in
      let token = part.trimmingCharacters(in: .whitespaces).lowercased()
      if let label = labels[token] { return label }
      let safe = String(token.prefix(24)).filter { character in
        character.unicodeScalars.allSatisfy { !CharacterSet.controlCharacters.contains($0) }
      }
      return safe.uppercased()
    }.filter { !$0.isEmpty }
    let display = parts.isEmpty ? "Ctrl + Shift + V" : parts.joined(separator: " + ")
    let state = enabled == false ? " · 已停用" : ""
    return "剪贴历史…（\(display)\(state)）"
  }

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

  /// IMK丢失Shift释放时，下一个不带任何组合修饰键的按键才可消费待切换手势。
  /// Shift仍按住的标点、数字、大写字母均属于组合输入，不能触发此补偿。
  static func shouldConsumeDeferredShiftToggle(
    armed: Bool,
    modifiers: NSEvent.ModifierFlags
  ) -> Bool {
    armed && modifiers.intersection([.shift, .command, .control, .option]).isEmpty
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

  /// 微信输入法式的临时英文组合：中文模式下按住 Shift 输入字母时，
  /// 先形成大写英文组合串，直到空格/回车再提交。
  static func isShiftEnglishCompositionCharacter(
    characters: String?,
    charactersIgnoringModifiers: String?,
    modifiers: NSEvent.ModifierFlags
  ) -> Bool {
    guard modifiers.contains(.shift),
      !modifiers.contains(.command),
      !modifiers.contains(.control),
      !modifiers.contains(.option),
      let characters,
      characters.count == 1,
      let ignoring = charactersIgnoringModifiers,
      ignoring.count == 1,
      let scalar = characters.unicodeScalars.first,
      let ignoringScalar = ignoring.unicodeScalars.first
    else {
      return false
    }
    return scalar.value >= 65 && scalar.value <= 90
      && ignoringScalar.value >= 97 && ignoringScalar.value <= 122
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
