import Foundation

@main
enum InputiaPredictionPresentationSelfCheck {
  static func main() {
    var failures: [String] = []
    func check(_ name: String, _ passed: Bool) {
      print("\(name)=\(passed)")
      if !passed { failures.append(name) }
    }

    var presentation = InputiaPredictionPresentation()
    check("predictionStartsClosed", presentation.currentToken == nil && presentation.deadline == nil
      && !presentation.isCurrent(1, now: 0))
    guard let first = presentation.begin(now: 100) else { fatalError("valid time rejected") }
    check("predictionOpensForFiveSeconds", presentation.currentToken == first
      && presentation.deadline == 105 && presentation.isCurrent(first, now: 100))
    check("predictionRejectsTimeBeforeCommit", !presentation.isCurrent(first, now: 99.999))
    check("predictionVisibleBeforeDeadline", presentation.isCurrent(first, now: 104.999))
    check("predictionStopsAtDeadline", !presentation.isCurrent(first, now: 105))
    check("predictionStopsAfterDeadline", !presentation.isCurrent(first, now: 105.001))

    let deadline = presentation.deadline
    for now in stride(from: 100.0, through: 104.9, by: 0.1) {
      _ = presentation.isCurrent(first, now: now)
    }
    check("predictionPollingDoesNotExtendDeadline", presentation.deadline == deadline
      && !presentation.isCurrent(first, now: 105))
    check("predictionEarlyTimerCannotClose", !presentation.expire(first, now: 104.999)
      && presentation.isCurrent(first, now: 104.999))
    check("predictionDeadlineTimerCloses", presentation.expire(first, now: 105)
      && presentation.currentToken == nil && presentation.deadline == nil)
    check("predictionExpiredReplyCannotReopen", !presentation.isCurrent(first, now: 105)
      && !presentation.expire(first, now: 106))

    guard let old = presentation.begin(now: 200), let current = presentation.begin(now: 202) else {
      fatalError("valid time rejected")
    }
    check("predictionNewCommitHasNewToken", old != current && presentation.deadline == 207)
    check("predictionOldReplyCannotReplaceNewSession", !presentation.isCurrent(old, now: 203)
      && presentation.isCurrent(current, now: 203))
    check("predictionOldTimerCannotCloseNewSession", !presentation.expire(old, now: 205)
      && presentation.currentToken == current && presentation.isCurrent(current, now: 205))
    presentation.invalidate()
    check("predictionExplicitCloseInvalidatesPendingReply", presentation.currentToken == nil
      && presentation.deadline == nil && !presentation.isCurrent(current, now: 205))
    check("predictionClosedTimerCannotReopen", !presentation.expire(current, now: 207)
      && presentation.currentToken == nil)
    guard let next = presentation.begin(now: 208) else { fatalError("valid time rejected") }
    check("predictionNewSessionAfterCloseRejectsOldToken", next != current
      && !presentation.isCurrent(current, now: 209) && presentation.isCurrent(next, now: 209))
    check("predictionRejectsNonFiniteRead", !presentation.isCurrent(next, now: .nan)
      && !presentation.isCurrent(next, now: .infinity))
    check("predictionInvalidTimerCannotClose", !presentation.expire(next, now: .infinity)
      && presentation.isCurrent(next, now: 209))
    for (index, invalid) in [Double.nan, .infinity, -1, .greatestFiniteMagnitude].enumerated() {
      check("predictionInvalidBeginCloses_\(index)", presentation.begin(now: invalid) == nil
        && presentation.currentToken == nil && presentation.deadline == nil)
    }

    print("predictionPresentationSelfCheckPassed=\(failures.isEmpty)")
    if !failures.isEmpty { exit(1) }
  }
}
