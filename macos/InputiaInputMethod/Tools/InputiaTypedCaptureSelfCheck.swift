import Foundation

@main
struct TypedCaptureSelfCheck {
  static func main() {
    // 首键前已有 broker token，后续 nihao marked 键均无新snapshot，仍保留原token。
    var origin: String? = nil
    for (i, _) in "nihao ".enumerated() {
      origin = InputiaTypedOriginLifetime.retain(origin, existingAllowed: origin != nil,
        candidate: i == 0 ? "broker-origin-before-n" : nil, boundary: false)
      precondition(origin == "broker-origin-before-n")
    }
    precondition(InputiaTypedOriginLifetime.retain(origin, existingAllowed: true,
      candidate: "old-token", boundary: true) == nil)
    precondition(InputiaTypedOriginLifetime.retain(origin, existingAllowed: false,
      candidate: Optional<String>.none, boundary: false) == nil)
    precondition(InputiaTypedOriginLifetime.retain(Optional<String>.none, existingAllowed: false,
      candidate: Optional<String>.none, boundary: false) == nil)
    precondition(!InputiaTypedCaptureState.invalidatesOrigin(nil))
    precondition(!InputiaTypedCaptureState.invalidatesOrigin("typed_capture_disabled_or_changed"))
    precondition(!InputiaTypedCaptureState.invalidatesOrigin("target_query_timeout"))
    precondition(InputiaTypedCaptureState.invalidatesOrigin("target_unknown"))
    precondition(InputiaTypedCaptureState.invalidatesOrigin("target_process_changed"))
    var state = InputiaTypedCaptureState()
    precondition(state.admit(identity: "a", start: 0, end: 1, text: "中", now: 0) == nil)
    state.policy(enabled: true, epoch: 1, server: "s", started: 1, now: 1)
    let first = state.admit(identity: "a", start: 0, end: 1, text: "中", now: 1)
    state.finish()
    precondition(first != nil)
    precondition(state.admit(identity: "a", start: 1, end: 2, text: "文", now: 1.1) == first)
    state.finish()
    precondition(state.admit(identity: "b", start: 2, end: 3, text: "字", now: 1.2) != first)
    state.finish()
    state.resetSegment() // 取消、离开字段均不产生正文事件。
    let afterCancel = state.admit(identity: "a", start: 2, end: 3, text: "。", now: 1.3)
    state.finish()
    precondition(afterCancel != first)
    precondition(state.admit(identity: "a", start: 3, end: 4, text: "新", now: 1.4) != afterCancel)
    state.finish()
    precondition(state.admit(identity: "a", start: 4, end: 5, text: "字", now: 4) == nil)
    state.policy(enabled: true, epoch: 1, server: "s", started: 1, now: 5)
    precondition(!state.enabled)
    state.policy(enabled: true, epoch: 2, server: "s", started: 10, now: 10)
    for i in 0..<4 { precondition(state.admit(identity: "a", start: i, end: i+1, text: "字", now: 10) != nil) }
    precondition(state.admit(identity: "a", start: 4, end: 5, text: "字", now: 10) == nil)
    state.policy(enabled: false, epoch: 3, server: "s", started: 10, now: 10)
    state.finish()
    precondition(state.admit(identity: "a", start: 0, end: 1, text: "字", now: 10) == nil)
    state.invalidate()
    precondition(InputiaTypedCaptureState.canBuffer(bytes: [1000, 1000], incoming: 2096, appending: false))
    precondition(!InputiaTypedCaptureState.canBuffer(bytes: [1000, 1000], incoming: 2097, appending: true))
    precondition(!InputiaTypedCaptureState.canBuffer(bytes: [1, 1, 1, 1], incoming: 1, appending: false))
    precondition(InputiaTypedCaptureState.canBuffer(bytes: [1, 1, 1, 1], incoming: 1, appending: true))
    precondition(!InputiaTypedCaptureState.canBuffer(bytes: [], incoming: 4097, appending: false))
    print("typedCaptureSelfCheck=true segmentation=true cancel=true expired=true disabled=true bounded=true fastMarkedOrigin=true captureFailureIsolation=true")
  }
}
