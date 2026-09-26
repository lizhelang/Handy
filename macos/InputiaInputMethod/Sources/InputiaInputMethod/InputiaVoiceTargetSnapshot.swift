import Cocoa
import InputMethodKit
import Carbon

#if INPUTIA_PAIRED_BUILD
enum InputiaVoiceTargetSnapshot {
  static let failedPreCaptureBackoffSeconds: TimeInterval = 2
  static var lastCaptureFailureReason = "service_unavailable"
  static func allowsHistoryOnlyCapture(reason: String) -> Bool { reason == "field_unobservable" }
  static func remainingQueryTimeout(now: TimeInterval, deadline: TimeInterval) -> Float? {
    now < deadline ? Float(min(0.05, deadline - now)) : nil
  }
  enum DispatchDecision { case dispatch(IMKTextInput), pending }
  final class Snapshot {
    let inputiaTarget: InputiaVoiceTarget
    var targetID: String { inputiaTarget.target_id }
    var controllerID: String { inputiaTarget.controller_id }
    var activationGeneration: UInt64 { inputiaTarget.activation_generation }
    let compositionGeneration: UInt64
    let localSelectionGeneration: UInt64
    let createdAt = ProcessInfo.processInfo.systemUptime
    let expiresAt: TimeInterval
    let permissionEpoch: UInt64
    private weak var clientObject: AnyObject?
    private let identity: ObjectIdentifier
    private let selectedRange: NSRange
    init(target: InputiaVoiceTarget, client: IMKTextInput, selection: NSRange,
      compositionGeneration: UInt64, localSelectionGeneration: UInt64, deadline: TimeInterval, permissionEpoch: UInt64) {
      inputiaTarget = target; clientObject = client as AnyObject; identity = ObjectIdentifier(client as AnyObject)
      selectedRange = selection; self.compositionGeneration = compositionGeneration
      self.localSelectionGeneration = localSelectionGeneration
      expiresAt = deadline; self.permissionEpoch = permissionEpoch
    }
    var reusableForShortcut: Bool {
      ProcessInfo.processInfo.systemUptime < expiresAt && InputiaPermissionLifecycle.shared.permits(permissionEpoch)
    }
    func isCurrentForShortcut(client: IMKTextInput?, controllerID: String, activationGeneration: UInt64,
      isSensitiveApp: (String, String?) -> Bool, windowTitle: (String) -> InputiaWindowTitleQuery.Result) -> Bool {
      guard Thread.isMainThread, reusableForShortcut, controllerID == self.controllerID,
        activationGeneration == self.activationGeneration, !IsSecureEventInputEnabled(),
        let client, let original = clientObject, ObjectIdentifier(client as AnyObject) == identity,
        ObjectIdentifier(original) == identity, client.bundleIdentifier() == inputiaTarget.source_app,
        Self.selectionMatches(selectedRange, client.selectedRange()) else { return false }
      return true
    }
    /// 键入来源仅放宽自身编辑导致的选区变化；原 AX 字段/焦点仍由服务验证。
    func isCurrentForTypedOrigin(client: IMKTextInput?, controllerID: String, activationGeneration: UInt64) -> Bool {
      guard Thread.isMainThread, reusableForShortcut, inputiaTarget.field_id != nil,
        controllerID == self.controllerID, activationGeneration == self.activationGeneration,
        !IsSecureEventInputEnabled(), let client, let original = clientObject,
        ObjectIdentifier(client as AnyObject) == identity, ObjectIdentifier(original) == identity,
        client.bundleIdentifier() == inputiaTarget.source_app else { return false }
      return true
    }
    private static func selectionMatches(_ captured: NSRange, _ current: NSRange) -> Bool {
      captured.location != NSNotFound && current == captured
    }
    func dispatchDecision(delivery: InputiaVoiceDelivery, client: IMKTextInput?, controllerID: String,
      activationGeneration: UInt64, latestComposing: String,
      isSensitiveApp: (String, String?) -> Bool, windowTitle: (String) -> InputiaWindowTitleQuery.Result) -> DispatchDecision {
      guard delivery.target_id == targetID, latestComposing.isEmpty,
        isCurrentForShortcut(client: client, controllerID: controllerID, activationGeneration: activationGeneration,
          isSensitiveApp: isSensitiveApp, windowTitle: windowTitle), let client,
        client.markedRange().location == NSNotFound || client.markedRange().length == 0 else { return .pending }
      return .dispatch(client)
    }
  }
  static func isWithinDispatchDeadline(_ delivery: InputiaVoiceDelivery) -> Bool { ProcessInfo.processInfo.systemUptime < delivery.dispatchDeadline }
  static func preCaptureRetryDeadline(after failureTime: TimeInterval) -> TimeInterval { failureTime + failedPreCaptureBackoffSeconds }
  static func shouldAttemptPreCapture(now: TimeInterval, retryDeadline: TimeInterval?) -> Bool { now >= (retryDeadline ?? 0) }
  static func validRange(_ range: NSRange) -> NSRange? { range.location == NSNotFound ? nil : range }
  static func rangeSignature(_ range: NSRange) -> UInt64 {
    (UInt64(UInt32(truncatingIfNeeded: range.location)) << 32) | UInt64(UInt32(truncatingIfNeeded: range.length))
  }
  static func rangeSignature(_ range: CFRange) -> UInt64 { rangeSignature(NSRange(location: range.location, length: range.length)) }
}
#endif

/// 仅一个后台窗口枚举任务；超时不会允许继续积压任务，也不复用旧窗口标题。
final class InputiaWindowTitleQuery {
  enum Result { case ready(String?), unavailable }
  private let lock = NSLock()
  private let queue = DispatchQueue(label: "Inputia.window-title", qos: .userInitiated)
  private var busy = false

  func read(timeout: TimeInterval = 0.025, lookup: @escaping () -> Result) -> Result {
    lock.lock()
    guard !busy else { lock.unlock(); return .unavailable }
    busy = true
    lock.unlock()
    let reply = Reply()
    queue.async {
      let value = lookup()
      reply.lock.lock(); reply.value = value; reply.lock.unlock()
      self.lock.lock(); self.busy = false; self.lock.unlock()
      reply.done.signal()
    }
    guard reply.done.wait(timeout: .now() + timeout) == .success else { return .unavailable }
    reply.lock.lock(); defer { reply.lock.unlock() }
    return reply.value
  }

  private final class Reply {
    let lock = NSLock()
    let done = DispatchSemaphore(value: 0)
    var value: Result = .unavailable
  }
}

struct InputiaAppContext: Equatable {
  let bundleId: String
  let windowTitle: String?
  var windowTitleAvailable = true
}

enum InputiaSecureDirectPolicy {
  private static let secureBundleIds: Set<String> = [
    "com.apple.SecurityAgent",
  ]

  static func shouldUseSecureDirectMode(context: InputiaAppContext) -> Bool {
    secureBundleIds.contains(context.bundleId)
  }
}
