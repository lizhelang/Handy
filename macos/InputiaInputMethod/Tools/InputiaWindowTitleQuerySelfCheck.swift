import Foundation
@main struct Check {
 static func main() {
  let query = InputiaWindowTitleQuery()
  precondition(!InputiaSecureDirectPolicy.shouldUseSecureDirectMode(context: InputiaAppContext(bundleId: "com.apple.Safari", windowTitle: nil, windowTitleAvailable: false)))
  precondition(!InputiaSecureDirectPolicy.shouldUseSecureDirectMode(context: InputiaAppContext(bundleId: "com.apple.TextEdit", windowTitle: nil)))
  precondition(InputiaSecureDirectPolicy.shouldUseSecureDirectMode(context: InputiaAppContext(bundleId: "com.apple.SecurityAgent", windowTitle: nil)))
  let release = DispatchSemaphore(value: 0)
  let finished = DispatchSemaphore(value: 0)
  let start = Date()
  if case .unavailable = query.read(timeout: 0.01, lookup: { release.wait(); finished.signal(); return .ready("old") }) {} else { fatalError("blocked lookup must time out") }
  precondition(Date().timeIntervalSince(start) < 0.2)
  if case .unavailable = query.read(lookup: { fatalError("must not queue while busy") }) {} else { fatalError("busy must reject") }
  release.signal(); precondition(finished.wait(timeout: .now()+1) == .success)
  var recovered = false
  for _ in 0..<100 {
   if case .ready(let value) = query.read(lookup: { .ready("new") }) { precondition(value == "new"); recovered = true; break }
   Thread.sleep(forTimeInterval:0.001)
  }
  precondition(recovered)
  if case .ready(let value) = query.read(lookup: { .ready(nil) }) { precondition(value == nil) } else { fatalError("empty completed lookup is valid") }
  print("windowQueryCheck=true timeout_bounded=true no_backlog=true late_result_discarded=true recovery=true")
 }
}
