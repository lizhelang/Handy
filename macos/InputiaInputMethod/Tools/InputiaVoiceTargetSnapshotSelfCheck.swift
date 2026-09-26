import Darwin
import Foundation

@main
struct InputiaVoiceTargetSnapshotSelfCheck {
  static func main() {
    #if INPUTIA_PAIRED_BUILD
    let range = NSRange(location: 42, length: 3)
    let cfRange = CFRange(location: 42, length: 3)
    let invalid = NSRange(location: NSNotFound, length: 0)
    let delivery = InputiaVoiceDelivery(
      operation_id: "op-1",
      session_id: "session-1",
      item_id: "item-1",
      revision: 1,
      policy_epoch: 2,
      target_id: "target-1",
      text: "ok",
      dispatchDeadline: ProcessInfo.processInfo.systemUptime + 0.5
    )
    let expired = InputiaVoiceDelivery(
      operation_id: "op-2",
      session_id: "session-1",
      item_id: "item-1",
      revision: 1,
      policy_epoch: 2,
      target_id: "target-1",
      text: "ok",
      dispatchDeadline: ProcessInfo.processInfo.systemUptime - 0.5
    )
    let failureTime = ProcessInfo.processInfo.systemUptime
    let retryDeadline = InputiaVoiceTargetSnapshot.preCaptureRetryDeadline(after: failureTime)

    let checks: [(String, Bool)] = [
      ("offlineBasicTypingAllowed", !InputiaSecureDirectPolicy.shouldUseSecureDirectMode(context:
        InputiaAppContext(bundleId: "com.apple.TextEdit", windowTitle: nil, windowTitleAvailable: false))),
      ("secureAgentStillPassesThrough", InputiaSecureDirectPolicy.shouldUseSecureDirectMode(context:
        InputiaAppContext(bundleId: "com.apple.SecurityAgent", windowTitle: nil, windowTitleAvailable: false))),
      ("queryBudgetCapsSingleCall", InputiaVoiceTargetSnapshot.remainingQueryTimeout(now: 10, deadline: 11) == 0.05),
      ("queryBudgetShrinksNearDeadline", abs((InputiaVoiceTargetSnapshot.remainingQueryTimeout(now: 10, deadline: 10.01) ?? 0) - 0.01) < 0.0001),
      ("queryBudgetRejectsExpired", InputiaVoiceTargetSnapshot.remainingQueryTimeout(now: 11, deadline: 11) == nil),
      ("validRangeKeepsSelection", InputiaVoiceTargetSnapshot.validRange(range) == range),
      ("invalidRangeRejected", InputiaVoiceTargetSnapshot.validRange(invalid) == nil),
      ("nsRangeSignatureMatchesCFRange", InputiaVoiceTargetSnapshot.rangeSignature(range) == InputiaVoiceTargetSnapshot.rangeSignature(cfRange)),
      ("freshDeliveryWithinDeadline", InputiaVoiceTargetSnapshot.isWithinDispatchDeadline(delivery)),
      ("expiredDeliveryRejected", !InputiaVoiceTargetSnapshot.isWithinDispatchDeadline(expired)),
      ("preCaptureBackoffBlocksImmediateRetry", !InputiaVoiceTargetSnapshot.shouldAttemptPreCapture(now: failureTime + 0.25, retryDeadline: retryDeadline)),
      ("preCaptureBackoffAllowsRetryAtBoundary", InputiaVoiceTargetSnapshot.shouldAttemptPreCapture(now: retryDeadline, retryDeadline: retryDeadline)),
      ("preCaptureBackoffIsBounded", retryDeadline - failureTime == InputiaVoiceTargetSnapshot.failedPreCaptureBackoffSeconds),
      ("preCaptureBackoffAllowsNoPriorFailure", InputiaVoiceTargetSnapshot.shouldAttemptPreCapture(now: failureTime, retryDeadline: nil)),
      ("missingPermissionDisablesVoice", !InputiaVoiceTargetSnapshot.allowsHistoryOnlyCapture(reason: "accessibility_permission_required")),
      ("sensitiveAndUnknownFailuresRejectCapture", ["secure_input_enabled", "secure_text_field", "focused_application_mismatch", "unknown"].allSatisfy {
        !InputiaVoiceTargetSnapshot.allowsHistoryOnlyCapture(reason: $0)
      }),
    ]
    for (name, ok) in checks {
      print("\(name)=\(ok)")
    }
    let passed = checks.allSatisfy { $0.1 }
    print("inputiaVoiceTargetSnapshotSelfCheck=\(passed)")
    exit(passed ? 0 : 1)
    #else
    print("inputiaVoiceTargetSnapshotSelfCheck=skipped pairedBuild=false")
    exit(0)
    #endif
  }
}
