import ApplicationServices
import Cocoa
import InputMethodKit

#if INPUTIA_PAIRED_BUILD
enum InputiaVoiceTargetSnapshot {
  static func allowsHistoryOnlyCapture(reason: String) -> Bool {
    ["accessibility_permission_required", "field_unobservable", "selection_unobservable",
     "field_observer_unavailable", "unsupported_focused_role"].contains(reason)
  }
  private(set) static var lastCaptureFailureReason = "unknown"
  enum DispatchDecision {
    case dispatch(IMKTextInput)
    case pending
  }

  final class Snapshot {
    let fieldID: String
    let targetID: String
    let hostInstance: String
    let controllerID: String
    let activationGeneration: UInt64
    let selectionGeneration: UInt64
    let compositionGeneration: UInt64
    let sourceApp: String?
    let inputiaTarget: InputiaVoiceTarget
    let createdAt: TimeInterval
    private weak var clientObject: AnyObject?
    private let clientIdentity: ObjectIdentifier
    private let initialSelectedRange: NSRange?
    private let initialAXSelectedRange: CFRange?
    private let focusedApplication: AXUIElement
    private let focusedElement: AXUIElement
    private let observation: Observation

    fileprivate init(
      fieldID: String,
      targetID: String,
      hostInstance: String,
      controllerID: String,
      activationGeneration: UInt64,
      selectionGeneration: UInt64,
      compositionGeneration: UInt64,
      sourceApp: String?,
      client: IMKTextInput,
      initialSelectedRange: NSRange?,
      initialAXSelectedRange: CFRange?,
      focusedApplication: AXUIElement,
      focusedElement: AXUIElement,
      observation: Observation
    ) {
      self.fieldID = fieldID
      self.targetID = targetID
      self.hostInstance = hostInstance
      self.controllerID = controllerID
      self.activationGeneration = activationGeneration
      self.selectionGeneration = selectionGeneration
      self.compositionGeneration = compositionGeneration
      self.sourceApp = sourceApp
      self.createdAt = ProcessInfo.processInfo.systemUptime
      self.clientObject = client as AnyObject
      self.clientIdentity = ObjectIdentifier(client as AnyObject)
      self.initialSelectedRange = initialSelectedRange
      self.initialAXSelectedRange = initialAXSelectedRange
      self.focusedApplication = focusedApplication
      self.focusedElement = focusedElement
      self.observation = observation
      self.inputiaTarget = InputiaVoiceTarget(
        target_id: targetID,
        host_instance: hostInstance,
        controller_id: controllerID,
        activation_generation: activationGeneration,
        field_id: fieldID,
        selection_generation: selectionGeneration,
        composition_generation: compositionGeneration,
        source_app: sourceApp
      )
    }

    func dispatchDecision(
      delivery: InputiaVoiceDelivery,
      client: IMKTextInput?,
      controllerID currentControllerID: String,
      activationGeneration currentActivationGeneration: UInt64,
      latestComposing: String,
      isSensitiveApp: (String, String?) -> Bool,
      windowTitle: (String) -> String?
    ) -> DispatchDecision {
      guard Thread.isMainThread,
            delivery.target_id == targetID,
            currentControllerID == controllerID,
            currentActivationGeneration == activationGeneration,
            latestComposing.isEmpty,
            !IsSecureEventInputEnabled(),
            let client,
            let originalClient = clientObject,
            ObjectIdentifier(client as AnyObject) == clientIdentity,
            ObjectIdentifier(originalClient) == clientIdentity,
            !observation.invalidated,
            let sourceApp,
            client.bundleIdentifier() == sourceApp,
            !isSensitiveApp(sourceApp, windowTitle(sourceApp)),
            rangesMatch(captured: initialSelectedRange, current: validRange(client.selectedRange())),
            rangesMatch(captured: initialAXSelectedRange, current: InputiaVoiceTargetSnapshot.selectedRange(from: focusedElement)),
            markedRangeIsClear(client.markedRange()),
            let currentFocus = InputiaVoiceTargetSnapshot.currentFocus(),
            CFEqual(currentFocus.application, focusedApplication),
            CFEqual(currentFocus.element, focusedElement),
            !InputiaVoiceTargetSnapshot.isSecureTextElement(currentFocus.element)
      else {
        return .pending
      }
      return .dispatch(client)
    }
  }

  static func capture(
    client: IMKTextInput,
    targetID: String,
    hostInstance: String,
    controllerID: String,
    activationGeneration: UInt64,
    compositionGeneration: UInt64,
    sourceApp: String
  ) -> Snapshot? {
    guard Thread.isMainThread else { return failCapture("not_main_thread") }
    guard AXIsProcessTrusted() else { return failCapture("accessibility_permission_required") }
    guard !IsSecureEventInputEnabled() else { return failCapture("secure_input_enabled") }
    guard let focus = currentFocus() else { return failCapture("field_unobservable") }
    guard focusedApplicationMatchesSource(focus.application, sourceApp: sourceApp) else {
      return failCapture("focused_application_mismatch")
    }
    guard isEditableTextElement(focus.element) else { return failCapture("unsupported_focused_role") }
    guard !isSecureTextElement(focus.element) else { return failCapture("secure_text_field") }
    let imkSelectedRange = validRange(client.selectedRange())
    let axRange = selectedRange(from: focus.element)
    guard imkSelectedRange != nil || axRange != nil else {
      return failCapture("selection_unobservable")
    }
    guard let observation = Observation(application: focus.application, element: focus.element) else {
      return failCapture("field_observer_unavailable")
    }
    return Snapshot(
      fieldID: UUID().uuidString,
      targetID: targetID,
      hostInstance: hostInstance,
      controllerID: controllerID,
      activationGeneration: activationGeneration,
      selectionGeneration: imkSelectedRange.map(rangeSignature) ?? axRange.map(rangeSignature) ?? 0,
      compositionGeneration: compositionGeneration,
      sourceApp: sourceApp,
      client: client,
      initialSelectedRange: imkSelectedRange,
      initialAXSelectedRange: axRange,
      focusedApplication: focus.application,
      focusedElement: focus.element,
      observation: observation
    )
  }

  private static func failCapture(_ reason: String) -> Snapshot? {
    lastCaptureFailureReason = reason
    NSLog("inputia_unified_voice_target_capture_failed reason=%@", reason)
    return nil
  }

  final class Observation {
    private var observer: AXObserver?
    private let state = ObservationState()
    private var refcon: UnsafeMutableRawPointer {
      Unmanaged.passUnretained(state).toOpaque()
    }
    var invalidated: Bool { state.invalidated }

    init?(application: AXUIElement, element: AXUIElement) {
      var pid: pid_t = 0
      guard AXUIElementGetPid(application, &pid) == .success else {
        return nil
      }
      var createdObserver: AXObserver?
      guard AXObserverCreate(pid, InputiaVoiceTargetSnapshot.observerCallback, &createdObserver) == .success,
            let createdObserver
      else {
        return nil
      }
      observer = createdObserver
      let source = AXObserverGetRunLoopSource(createdObserver)
      CFRunLoopAddSource(CFRunLoopGetMain(), source, .commonModes)
      guard add(createdObserver, application, kAXFocusedUIElementChangedNotification as CFString),
            add(createdObserver, element, kAXSelectedTextChangedNotification as CFString),
            add(createdObserver, element, kAXValueChangedNotification as CFString),
            add(createdObserver, element, kAXUIElementDestroyedNotification as CFString)
      else {
        CFRunLoopRemoveSource(CFRunLoopGetMain(), source, .commonModes)
        observer = nil
        return nil
      }
    }

    deinit {
      guard let observer else {
        return
      }
      CFRunLoopRemoveSource(CFRunLoopGetMain(), AXObserverGetRunLoopSource(observer), .commonModes)
    }

    private func add(_ observer: AXObserver, _ element: AXUIElement, _ notification: CFString) -> Bool {
      AXObserverAddNotification(observer, element, notification, refcon) == .success
    }
  }

  private final class ObservationState {
    var invalidated = false
  }

  private static let observerCallback: AXObserverCallback = { _, _, _, refcon in
    guard let refcon else {
      return
    }
    Unmanaged<ObservationState>.fromOpaque(refcon).takeUnretainedValue().invalidated = true
  }

  static func isWithinDispatchDeadline(_ delivery: InputiaVoiceDelivery) -> Bool {
    ProcessInfo.processInfo.systemUptime < delivery.dispatchDeadline
  }

  static func validRange(_ range: NSRange) -> NSRange? {
    guard range.location != NSNotFound else {
      return nil
    }
    return range
  }

  static func rangeSignature(_ range: NSRange) -> UInt64 {
    (UInt64(UInt32(truncatingIfNeeded: range.location)) << 32) | UInt64(UInt32(truncatingIfNeeded: range.length))
  }

  static func rangeSignature(_ range: CFRange) -> UInt64 {
    (UInt64(UInt32(truncatingIfNeeded: range.location)) << 32) | UInt64(UInt32(truncatingIfNeeded: range.length))
  }

  private static func rangesMatch(captured: NSRange?, current: NSRange?) -> Bool {
    switch (captured, current) {
    case (.none, .none):
      return true
    case (.some(let lhs), .some(let rhs)):
      return lhs.location == rhs.location && lhs.length == rhs.length
    default:
      return false
    }
  }

  private static func rangesMatch(captured: CFRange?, current: CFRange?) -> Bool {
    switch (captured, current) {
    case (.none, .none):
      return true
    case (.some(let lhs), .some(let rhs)):
      return lhs.location == rhs.location && lhs.length == rhs.length
    default:
      return false
    }
  }

  private static func markedRangeIsClear(_ range: NSRange) -> Bool {
    range.location == NSNotFound || range.length == 0
  }

  private static func currentFocus() -> (application: AXUIElement, element: AXUIElement)? {
    let system = AXUIElementCreateSystemWide()
    guard let application = copyElementAttribute(system, kAXFocusedApplicationAttribute as CFString) else {
      return nil
    }
    AXUIElementSetMessagingTimeout(application, 0.05)
    guard let element = copyElementAttribute(application, kAXFocusedUIElementAttribute as CFString) else {
      return nil
    }
    AXUIElementSetMessagingTimeout(element, 0.05)
    return (application, element)
  }

  private static func copyElementAttribute(_ element: AXUIElement, _ attribute: CFString) -> AXUIElement? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, attribute, &value) == .success,
          let value,
          CFGetTypeID(value) == AXUIElementGetTypeID()
    else {
      return nil
    }
    return (value as! AXUIElement)
  }

  private static func focusedApplicationMatchesSource(_ application: AXUIElement, sourceApp: String) -> Bool {
    var pid: pid_t = 0
    guard AXUIElementGetPid(application, &pid) == .success,
          let frontmost = NSWorkspace.shared.frontmostApplication,
          frontmost.processIdentifier == pid,
          frontmost.bundleIdentifier == sourceApp
    else {
      return false
    }
    return true
  }

  private static func selectedRange(from element: AXUIElement) -> CFRange? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, kAXSelectedTextRangeAttribute as CFString, &value) == .success,
          let rawValue = value,
          CFGetTypeID(rawValue) == AXValueGetTypeID()
    else {
      return nil
    }
    let axValue = rawValue as! AXValue
    guard AXValueGetType(axValue) == .cfRange else {
      return nil
    }
    var range = CFRange()
    guard AXValueGetValue(axValue, .cfRange, &range), range.location != kCFNotFound else {
      return nil
    }
    return range
  }

  private static func isSecureTextElement(_ element: AXUIElement) -> Bool {
    guard let subrole = stringAttribute(element, kAXSubroleAttribute as CFString) else {
      return false
    }
    return subrole == (kAXSecureTextFieldSubrole as String)
  }

  private static func isEditableTextElement(_ element: AXUIElement) -> Bool {
    guard let role = stringAttribute(element, kAXRoleAttribute as CFString) else {
      return false
    }
    let textRoles = [
      kAXTextFieldRole as String,
      kAXTextAreaRole as String,
      kAXComboBoxRole as String,
    ]
    guard textRoles.contains(role) else {
      return false
    }
    return boolAttribute(element, kAXIsEditableAttribute as CFString) != false
  }

  private static func boolAttribute(_ element: AXUIElement, _ attribute: CFString) -> Bool? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, attribute, &value) == .success else {
      return nil
    }
    return value as? Bool
  }

  private static func stringAttribute(_ element: AXUIElement, _ attribute: CFString) -> String? {
    var value: CFTypeRef?
    guard AXUIElementCopyAttributeValue(element, attribute, &value) == .success else {
      return nil
    }
    return value as? String
  }
}
#endif
