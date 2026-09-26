import Foundation

/// marked 组合过程可以保留既有原字段证明；边界事件绝不采用旧候选。
enum InputiaTypedOriginLifetime {
  static func retain<T>(_ existing: T?, existingAllowed: Bool, candidate: T?, boundary: Bool) -> T? {
    guard !boundary else { return nil }
    return existingAllowed ? (existing ?? candidate) : candidate
  }
}

/// 只保存分段位置与短期策略，不保存键入正文。无租约时默认关闭。
struct InputiaTypedCaptureState {
  private(set) var epoch: UInt64 = 0
  private(set) var deadline: TimeInterval = 0
  private(set) var enabled = false
  private(set) var pending = 0
  private var identity: String?
  private var segmentID: String?
  private var nextLocation: Int?
  private var lastCommit: TimeInterval = 0
  private var server: String?
  mutating func policy(enabled: Bool, epoch: UInt64, server: String, started: TimeInterval, now: TimeInterval) {
    if self.epoch != epoch || self.server != server || !enabled || now >= started + 3 { resetSegment() }
    self.epoch = epoch; self.server = server
    self.enabled = enabled && now < started + 3
    deadline = self.enabled ? started + 3 : 0
  }
  mutating func invalidate() { enabled = false; deadline = 0; resetSegment() }
  mutating func resetSegment() { identity = nil; segmentID = nil; nextLocation = nil; lastCommit = 0 }
  mutating func finish() { pending = max(0, pending - 1) }
  static func invalidatesOrigin(_ code: String?) -> Bool {
    guard let code else { return false }
    return ["target_unknown", "target_expired", "target_owner_mismatch", "target_process_changed",
      "target_source_changed", "target_sensitive_source", "focused_application_mismatch"].contains(code)
  }
  static func canBuffer(bytes: [Int], incoming: Int, appending: Bool) -> Bool {
    incoming > 0 && incoming <= 4096 && bytes.reduce(0, +) + incoming <= 4096
      && (appending || bytes.count < 4)
  }
  mutating func admit(identity: String, start: Int, end: Int, text: String, now: TimeInterval) -> String? {
    guard enabled, now < deadline, pending < 4, !text.isEmpty, text.utf8.count <= 4096,
      start >= 0, end >= start, end - start == text.utf16.count else { resetSegment(); return nil }
    if self.identity != identity || nextLocation != start || now - lastCommit > 2 { segmentID = UUID().uuidString }
    let result = segmentID ?? UUID().uuidString
    self.identity = identity; segmentID = result; nextLocation = end; lastCommit = now; pending += 1
    if text.last.map({ ".!?。！？\n\r".contains($0) }) == true { resetSegment() }
    return result
  }
}

#if INPUTIA_PAIRED_BUILD
/// 缓存仅由主 Actor 使用；短批次最多 4 KiB，单个请求在途，失败直接丢弃。
@MainActor
final class InputiaTypedCapture {
  static let shared = InputiaTypedCapture()
  private struct Batch {
    var command: InputiaTypedCaptureCommand
    let server: String
    let deadline: TimeInterval
  }
  private var state = InputiaTypedCaptureState()
  private var timer: Timer?
  private var flushTimer: Timer?
  private var polling = false
  private var sending = false
  private var batches: [Batch] = []
  private var generation: UInt64 = 0
  private var policyServer: String?
  var invalidOrigin: ((String) -> Void)?
  func activate() {
    state.resetSegment()
    guard timer == nil else { return }
    generation &+= 1
    poll()
    timer = Timer(timeInterval: 1.5, repeats: true) { _ in
      MainActor.assumeIsolated { InputiaTypedCapture.shared.poll() }
    }
    flushTimer = Timer(timeInterval: 0.25, repeats: true) { _ in
      MainActor.assumeIsolated { InputiaTypedCapture.shared.flush() }
    }
    if let timer { RunLoop.main.add(timer, forMode: .common) }
    if let flushTimer { RunLoop.main.add(flushTimer, forMode: .common) }
  }
  func deactivate() {
    generation &+= 1; timer?.invalidate(); timer = nil
    flushTimer?.invalidate(); flushTimer = nil; state.invalidate(); batches.removeAll()
  }
  func resetSegment() { state.resetSegment(); batches.removeAll() }
  private func poll() {
    guard !polling else { return }
    polling = true
    let started = ProcessInfo.processInfo.systemUptime
    let ticket = generation
    InputiaVoiceInputLauncher.typedCapture(.init(kind: "policy"), deadline: started + 3) { reply in
      MainActor.assumeIsolated {
        self.polling = false
        guard ticket == self.generation else { return }
        guard let reply else { self.state.invalidate(); self.batches.removeAll(); return }
        if !reply.enabled || reply.epoch != self.state.epoch || reply.server_instance != self.policyServer {
          self.batches.removeAll()
        }
        self.policyServer = reply.server_instance
        self.state.policy(enabled: reply.enabled, epoch: reply.epoch, server: reply.server_instance,
          started: started, now: ProcessInfo.processInfo.systemUptime)
        if !self.state.enabled { self.batches.removeAll() }
      }
    }
  }
  func committed(text: String, draft: InputiaVoiceTarget, identity: String, start: Int, end: Int) {
    guard let segment = state.admit(identity: identity, start: start, end: end, text: text,
      now: ProcessInfo.processInfo.systemUptime) else { batches.removeAll(); return }
    state.finish()
    let appending = batches.last?.command.segment_id == segment
    guard let server = policyServer, InputiaTypedCaptureState.canBuffer(
      bytes: batches.map { $0.command.text?.utf8.count ?? 0 }, incoming: text.utf8.count, appending: appending) else {
      resetSegment(); return
    }
    if let last = batches.indices.last, batches[last].command.segment_id == segment,
      batches[last].command.draft == draft {
      batches[last].command.text = (batches[last].command.text ?? "") + text
    } else {
      guard batches.count < 4 else { resetSegment(); return }
      batches.append(Batch(command: InputiaTypedCaptureCommand(kind: "commit", capture_epoch: state.epoch,
        event_id: UUID().uuidString, segment_id: segment, text: text, draft: draft),
        server: server, deadline: state.deadline))
    }
  }
  private func flush() {
    guard state.enabled, ProcessInfo.processInfo.systemUptime < state.deadline else {
      state.invalidate(); batches.removeAll(); return
    }
    guard !sending, !batches.isEmpty else { return }
    let batch = batches.removeFirst()
    guard batch.server == policyServer, batch.command.capture_epoch == state.epoch,
      ProcessInfo.processInfo.systemUptime < batch.deadline else { resetSegment(); return }
    sending = true
    let ticket = generation
    InputiaVoiceInputLauncher.typedCapture(batch.command, deadline: batch.deadline,
      expectedServer: batch.server) { reply in
      MainActor.assumeIsolated {
        self.sending = false
        guard ticket == self.generation else { return }
        if reply == nil || reply?.saved != true {
          self.resetSegment()
          InputiaPersonalizationDiagnostics.record("typed_result", reply?.code ?? (reply == nil ? "no_reply" : "disabled"))
          if let code = reply?.code, InputiaTypedCaptureState.invalidatesOrigin(code), let id = batch.command.draft?.target_id {
            self.invalidOrigin?(id)
          }
        }
        if reply?.enabled == false { self.state.invalidate(); self.batches.removeAll() }
      }
    }
  }
}
#endif
