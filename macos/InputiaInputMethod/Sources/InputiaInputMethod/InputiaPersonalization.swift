import Foundation

/// 只保留本字段已确认上屏的短上下文，目标变化不能沿用前文。
struct InputiaPersonalContext {
  private(set) var targetID: String?
  private(set) var text = ""
  private(set) var generation: UInt64 = 0
  mutating func bind(_ id: String) {
    if targetID != id { reset(); targetID = id }
  }
  mutating func append(_ value: String) { text = String((text + value).suffix(64)); generation &+= 1 }
  mutating func reset() { targetID = nil; text = ""; generation &+= 1 }
  static func hasNativeLearning(candidateID: String) -> Bool {
    candidateID.hasPrefix("rime:") || candidateID.hasPrefix("rime-correction:")
  }
  static func consumedCode(_ code: String, length: Int) -> String? {
    guard length > 0, length <= code.count else { return nil }
    return String(code.prefix(length))
  }
  static func replyIsCurrent(ticket: UInt64, expectedTicket: UInt64, target: String?, expectedTarget: String,
    context: UInt64, expectedContext: UInt64, epoch: UInt64, expectedEpoch: UInt64) -> Bool {
    ticket == expectedTicket && target == expectedTarget && context == expectedContext && epoch == expectedEpoch
  }
  static func matchesFirstPage(page: Int, expectedPage: Int, ids: [String], expectedIDs: [String]) -> Bool {
    page == 0 && expectedPage == 0 && ids == expectedIDs
  }
  static func retainsPersonalIdentity(_ id: String, available: [String]) -> Bool { available.contains(id) }
  static func validOrder(_ ids: [String], candidates: [(String, String)]) -> Bool {
    let available = Set(candidates.map { $0.0 })
    return ids.count <= candidates.count && Set(ids).count == ids.count && ids.allSatisfy { available.contains($0) }
  }
}

#if INPUTIA_PAIRED_BUILD
/// 由 IMK 主线程拥有；所有认证连接与服务查询都异步，不改变 Rime 的线程归属。
final class InputiaPersonalization {
  struct View {
    let target: InputiaVoiceTarget
    let code: String
    let epoch: UInt64
    let candidates: [InputiaPersonalCandidate]
    let predictions: [InputiaPersonalPrediction]
    let generation: UInt64
  }
  struct Admission {
    let targetID: String
    let epoch: UInt64
    let ticket: UInt64
    let contextGeneration: UInt64
    let server: String
    let deadline: TimeInterval
  }
  struct Receipt {
    let command: InputiaPersonalCommand
    let server: String
    let epoch: UInt64
  }
  private(set) var context = InputiaPersonalContext()
  private(set) var view: View?
  private(set) var epoch: UInt64 = 0
  private var server: String?
  private var enabled = false
  private var deadline: TimeInterval = 0
  private var timer: Timer?
  private var policyBusy = false
  private var queryBusy = false
  private var feedbackCount = 0
  private var admissionBusy = false
  private var ticket: UInt64 = 0
  private var debounce: DispatchWorkItem?
  private var waitingQuery: DispatchWorkItem?
  var changed: (() -> Void)?
  var policyChanged: (() -> Void)?
  var allowed: Bool { enabled && ProcessInfo.processInfo.systemUptime < deadline }
  func start() {
    guard timer == nil else { return }
    poll()
    timer = Timer(timeInterval: 1.5, repeats: true) { [weak self] _ in self?.poll() }
    if let timer { RunLoop.main.add(timer, forMode: .common) }
  }
  func stop() { timer?.invalidate(); timer = nil; enabled = false; reset() }
  func reset() { context.reset(); invalidateView() }
  func invalidateView() {
    ticket &+= 1; debounce?.cancel(); debounce = nil
    waitingQuery?.cancel(); waitingQuery = nil; view = nil; changed?()
  }
  private func poll() {
    guard !policyBusy else { return }
    policyBusy = true
    let start = ProcessInfo.processInfo.systemUptime
    InputiaVoiceInputLauncher.personalization(.init(kind: "policy"), deadline: start + 3) { [weak self] reply in
      guard let self else { return }; self.policyBusy = false
      InputiaPersonalizationDiagnostics.record("policy", reply?.code ?? (reply == nil ? "no_reply" : "ok"),
        flags: (reply?.enabled == true ? 1 : 0) | (ProcessInfo.processInfo.systemUptime < start + 3 ? 2 : 0) | (self.timer != nil ? 4 : 0))
      guard self.timer != nil || self.allowed else { return }
      guard let reply, ProcessInfo.processInfo.systemUptime < start + 3 else {
        self.enabled = false; self.reset(); return
      }
      if self.epoch != reply.epoch || self.server != reply.server_instance || !reply.enabled { self.reset() }
      self.epoch = reply.epoch; self.server = reply.server_instance
      self.enabled = reply.enabled; self.deadline = start + 3
      self.policyChanged?()
      let expiry = self.deadline
      DispatchQueue.main.asyncAfter(deadline: .now() + max(0, expiry - ProcessInfo.processInfo.systemUptime)) { [weak self] in
        guard let self, self.deadline == expiry, !self.allowed else { return }
        self.reset()
      }
    }
  }
  func bind(_ target: InputiaVoiceTarget) {
    if context.targetID != target.target_id { reset(); context.bind(target.target_id) }
  }
  func query(target: InputiaVoiceTarget, code: String, candidates: [InputiaPersonalCandidate],
    completion: @escaping (View) -> Void) {
    guard allowed, target.field_id != nil, candidates.count <= 64 else { return }
    bind(target)
    ticket &+= 1
    let version = ticket
    let contextVersion = context.generation
    debounce?.cancel()
    let work = DispatchWorkItem { [weak self] in
      guard let self, self.allowed, self.ticket == version,
        self.context.generation == contextVersion, let server = self.server else { return }
      if self.queryBusy {
        self.waitingQuery?.cancel(); self.waitingQuery = self.debounce
        return
      }
      self.queryBusy = true
      let sentEpoch = self.epoch
      let command = InputiaPersonalCommand(kind: "query", target: target, learning_epoch: sentEpoch,
        input_code: code, context: self.context.text, context_id: target.target_id, candidates: candidates, limit: code.isEmpty ? 3 : 5)
      InputiaVoiceInputLauncher.personalization(command, deadline: self.deadline, expectedServer: server) { [weak self] reply in
        guard let self else { return }; self.queryBusy = false
        InputiaPersonalizationDiagnostics.record("query", reply?.code ?? (reply == nil ? "no_reply" : "ok"),
          flags: (reply?.enabled == true ? 1 : 0) | (self.ticket == version ? 2 : 0),
          count: reply?.result?.predictions?.count ?? 0)
        defer {
          if let waiting = self.waitingQuery {
            self.waitingQuery = nil
            DispatchQueue.main.async(execute: waiting)
          }
        }
        if let reply, !reply.enabled || reply.epoch != sentEpoch || reply.code != nil {
          self.enabled = false; self.reset(); return
        }
        guard self.allowed, InputiaPersonalContext.replyIsCurrent(ticket: self.ticket, expectedTicket: version,
          target: self.context.targetID, expectedTarget: target.target_id, context: self.context.generation,
          expectedContext: contextVersion, epoch: self.epoch, expectedEpoch: sentEpoch),
          let reply, reply.enabled, reply.epoch == sentEpoch, reply.server_instance == server,
          let result = reply.result, result.context_id == target.target_id else { return }
        let ids = result.ordered_ids ?? []
        guard InputiaPersonalContext.validOrder(ids, candidates: candidates.map { ($0.id, $0.text) }) else { return }
        let byID = Dictionary(candidates.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        let ordered = ids.compactMap { byID[$0] } + candidates.filter { !ids.contains($0.id) }
        let predictions = (result.predictions ?? []).filter { !$0.text.isEmpty && $0.text.count <= 64 }.prefix(5)
        let view = View(target: target, code: code, epoch: sentEpoch, candidates: ordered,
          predictions: Array(predictions), generation: version)
        self.view = view; completion(view)
      }
    }
    debounce = work
    DispatchQueue.main.asyncAfter(deadline: .now() + 0.12, execute: work)
  }
  func admissionIsCurrent(_ admission: Admission) -> Bool {
    allowed && epoch == admission.epoch && server == admission.server && ticket == admission.ticket
      && context.targetID == admission.targetID && context.generation == admission.contextGeneration
      && view?.generation == admission.ticket && ProcessInfo.processInfo.systemUptime < admission.deadline
  }
  func admit(_ prediction: InputiaPersonalPrediction, from selected: View,
    completion: @escaping (Admission?) -> Void) {
    guard allowed, !admissionBusy, selected.code.isEmpty, selected.epoch == epoch,
      selected.generation == ticket, view?.generation == ticket,
      selected.predictions.contains(prediction), context.targetID == selected.target.target_id,
      let server else { completion(nil); return }
    admissionBusy = true
    let receipt = Admission(targetID: selected.target.target_id, epoch: epoch, ticket: ticket,
      contextGeneration: context.generation, server: server,
      deadline: min(deadline, ProcessInfo.processInfo.systemUptime + 0.75))
    let command = InputiaPersonalCommand(kind: "admit", target: selected.target, learning_epoch: epoch,
      context: context.text, context_id: selected.target.target_id, text: prediction.text, prediction_id: prediction.id)
    InputiaVoiceInputLauncher.personalization(command, deadline: receipt.deadline, expectedServer: server) { [weak self] reply in
      guard let self else { completion(nil); return }
      self.admissionBusy = false
      guard let reply, reply.enabled, reply.epoch == receipt.epoch, reply.server_instance == server,
        reply.code == nil, reply.result?.admitted == true,
        reply.result?.prediction_id == prediction.id, reply.result?.context_id == receipt.targetID,
        self.admissionIsCurrent(receipt) else {
        self.reset()
        if let reply, !reply.enabled || reply.epoch != self.epoch { self.enabled = false }
        completion(nil); return
      }
      completion(receipt)
    }
  }

  func accepted(target: InputiaVoiceTarget, code: String, text: String, explicit: Bool, rank: Int,
    completion: @escaping () -> Void) -> Receipt? {
    guard allowed, !text.isEmpty, text.count <= 64, feedbackCount < 8, let server else { return nil }
    bind(target)
    let command = InputiaPersonalCommand(kind: "feedback", target: target, learning_epoch: epoch,
      input_code: code, context_id: target.target_id, event_id: UUID().uuidString, text: text,
      previous: context.text, explicit_selection: explicit, original_rank: rank, operation: "accept")
    let receipt = Receipt(command: command, server: server, epoch: epoch)
    context.append(text); invalidateView(); feedbackCount += 1
    InputiaVoiceInputLauncher.personalization(command, deadline: deadline, expectedServer: server) { [weak self] reply in
      guard let self else { return }; self.feedbackCount -= 1
      guard self.allowed, self.epoch == receipt.epoch, self.context.targetID == target.target_id,
        let reply, reply.enabled, reply.epoch == receipt.epoch, reply.code == nil else { self.reset(); return }
      completion()
    }
    return receipt
  }
  func undo(_ receipt: Receipt) {
    guard allowed, receipt.epoch == epoch, receipt.server == server, feedbackCount < 8 else { return }
    var command = receipt.command
    command.operation = "undo"
    // 同一个event_id引用刚确认的选择，而不是新建第二次选择。
    feedbackCount += 1
    InputiaVoiceInputLauncher.personalization(command, deadline: deadline, expectedServer: receipt.server) { [weak self] _ in
      guard let self else { return }; self.feedbackCount -= 1
    }
    reset()
  }
}
#endif
