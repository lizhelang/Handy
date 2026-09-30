import Foundation
import AppKit

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

/// 只组合一次原始 composition 中连续确认的片段；任何编辑或字段变化都截断链路。
struct InputiaPersonalPhraseAssembly {
  struct Phrase { let code: String; let text: String; let previous: String; let explicit: Bool; let originalRank: Int }
  private var targetID = ""
  private var schemaID = ""
  private var learningEpoch: UInt64 = 0
  private var originalCode = ""
  private var remaining = ""
  private var text = ""
  private var previous = ""
  private var nextStart = -1
  private var count = 0
  private var explicit = false
  private var originalRank = 0
  mutating func reset() { self = Self() }
  mutating func confirmed(target: String, schema: String, epoch: UInt64 = 0, composing: String, consumed: String,
    remaining next: String, text value: String, previous context: String, start: Int,
    explicit selectedExplicitly: Bool = false, rank: Int = 0) -> Phrase? {
    guard !consumed.isEmpty, composing.utf8.allSatisfy({ $0 < 128 }),
      composing.hasPrefix(consumed), String(composing.dropFirst(consumed.count)) == next,
      !value.isEmpty, start >= 0 else { reset(); return nil }
    if targetID != target || schemaID != schema || learningEpoch != epoch || remaining != composing || nextStart != start {
      reset(); targetID = target; schemaID = schema; learningEpoch = epoch; originalCode = composing; previous = context
    }
    text += value; remaining = next; count += 1; nextStart = start + value.utf16.count
    explicit = explicit || selectedExplicitly; originalRank = max(originalRank, rank)
    guard text.count <= 64, originalCode.count <= 128 else { reset(); return nil }
    guard next.isEmpty else { return nil }
    let phrase = count > 1 ? Phrase(code: originalCode, text: text, previous: previous,
      explicit: explicit, originalRank: originalRank) : nil
    reset(); return phrase
  }
}

/// 与主机共用的事件分类；纯Shift变化是普通快打的一部分，不能撤销先前选词。
enum InputiaPersonalDeferredEventPolicy {
  enum Kind { case keyDown, keyUp, flagsChanged, other }
  static func shouldQueue(_ event: NSEvent, candidateNavigation: Bool) -> Bool {
    let kind: Kind
    switch event.type {
    case .keyDown: kind = .keyDown
    case .keyUp: kind = .keyUp
    case .flagsChanged: kind = .flagsChanged
    default: kind = .other
    }
    return shouldQueue(kind: kind, keyCode: event.keyCode,
      blockingModifiers: !event.modifierFlags.intersection([.command, .control, .option]).isEmpty,
      text: event.type == .keyDown ? event.characters : nil, candidateNavigation: candidateNavigation)
  }
  static func shouldQueue(kind: Kind, keyCode: UInt16, blockingModifiers: Bool,
    text: String?, candidateNavigation: Bool) -> Bool {
    guard !blockingModifiers else { return false }
    switch kind {
    case .flagsChanged: return [UInt16(56), 60].contains(keyCode)
    case .keyUp: return ![UInt16(54), 55, 58, 59, 61, 62].contains(keyCode)
    case .keyDown:
      guard !candidateNavigation,
        ![UInt16(51), 53, 36, 76, 48, 115, 116, 117, 119, 121, 123, 124, 125, 126].contains(keyCode),
        let text, !text.isEmpty, text.utf16.count <= 64 else { return false }
      return text.unicodeScalars.allSatisfy {
        !CharacterSet.controlCharacters.contains($0) && !(0xF700...0xF8FF).contains($0.value)
      }
    case .other: return false
    }
  }
  static func isPrintable(_ event: NSEvent) -> Bool {
    shouldQueue(event, candidateNavigation: false) && event.type == .keyDown
  }
  /// 仅推演一定由本IMK消费的普通拼音编辑；空composition上的宿主命令保留物理事件。
  static func canQueueEditing(_ event: NSEvent, after queued: [NSEvent], chineseMode: Bool) -> Bool {
    guard chineseMode, event.type == .keyDown,
      event.modifierFlags.intersection([.command, .control, .option, .shift]).isEmpty,
      [UInt16(51), 53, 116, 121, 125, 126].contains(event.keyCode) else { return false }
    // 独立Shift可能切入英文，后续文字将由宿主直插；此时不能消费其物理编辑键。
    guard !queued.contains(where: { $0.type == .flagsChanged || [UInt16(56), 60].contains($0.keyCode)
      || $0.modifierFlags.contains(.shift) }) else { return false }
    var letters = 0
    for prior in queued where prior.type == .keyDown || prior.type == .flagsChanged {
      if prior.type == .flagsChanged { letters = 0; continue }
      guard prior.modifierFlags.intersection([.command, .control, .option, .shift]).isEmpty else {
        letters = 0; continue
      }
      if let text = prior.characters, !text.isEmpty,
        text.utf8.allSatisfy({ (97...122).contains($0) }) { letters += text.count }
      else if prior.keyCode == 51 { letters = max(0, letters - 1) }
      else if ![UInt16(116), 121, 125, 126].contains(prior.keyCode) { letters = 0 }
    }
    return letters > 0
  }
}

/// 专用mode只接个人RPC结果；等待原物理控制键时不泵AppKit事件，后来的键不能抢跑。
enum InputiaPersonalMainDelivery {
  static let waitMode = RunLoop.Mode("InputiaPersonalAdmissionWait")
  static func deliver(_ completion: @escaping () -> Void) {
    RunLoop.main.perform(inModes: [.default, .common, waitMode], block: completion)
    CFRunLoopWakeUp(CFRunLoopGetMain())
  }
}

final class InputiaPersonalBoundaryWait {
  enum Result { case completed, timedOut, invalidated, reentrant }
  private(set) var isWaiting = false
  func wait(deadline: TimeInterval, pending: () -> Bool, valid: () -> Bool) -> Result {
    guard !isWaiting else { return .reentrant }
    isWaiting = true
    // Timer令专用mode保持休眠；完成回调主动唤醒，禁止空转或延长原750ms预算。
    let timer = Timer(timeInterval: max(0.001, deadline - ProcessInfo.processInfo.systemUptime), repeats: false) { _ in }
    RunLoop.main.add(timer, forMode: InputiaPersonalMainDelivery.waitMode)
    defer { timer.invalidate(); isWaiting = false }
    while pending() {
      guard valid() else { return .invalidated }
      let now = ProcessInfo.processInfo.systemUptime
      guard now < deadline else { return .timedOut }
      RunLoop.main.run(mode: InputiaPersonalMainDelivery.waitMode,
        before: Date().addingTimeInterval(min(0.01, deadline - now)))
    }
    return valid() ? .completed : .invalidated
  }
}

/// 异步准入期间暂存本输入会话后续文本；不注入系统事件，不跨目标或世代转交。
struct InputiaPersonalInputDeferral<Event> {
  enum Decision { case queued, flush, discard }
  private(set) var token: UInt64?
  private(set) var events: [Event] = []
  private var nextToken: UInt64 = 0
  private(set) var deadline: TimeInterval = 0
  static var timeout: TimeInterval { 0.75 }
  static var capacity: Int { 32 }
  var isPending: Bool { token != nil }
  /// nil/拒绝/过期均无字段证明，同IMK代理和相同选区不能替代服务端原字段核验。
  static func allowsReplay(proofReady: Bool, proofDeadline: TimeInterval?, now: TimeInterval,
    leaseMatches: Bool, scopeMatches: Bool) -> Bool {
    guard let proofDeadline else { return false }
    return proofReady && now < proofDeadline && leaseMatches && scopeMatches
  }
  mutating func begin(now: TimeInterval) -> UInt64? {
    guard token == nil else { return nil }
    nextToken &+= 1; token = nextToken; deadline = now + Self.timeout; events.removeAll()
    return nextToken
  }
  mutating func offer(_ event: Event, now: TimeInterval, scopeMatches: Bool, boundary: Bool) -> Decision {
    guard token != nil, scopeMatches else { return .discard }
    guard !boundary, now < deadline, events.count < Self.capacity else { return .flush }
    events.append(event); return .queued
  }
  func expired(_ expected: UInt64, now: TimeInterval) -> Bool { token == expected && now >= deadline }
  mutating func finish(_ expected: UInt64) -> [Event]? {
    guard token == expected else { return nil }
    let buffered = events; token = nil; events.removeAll(); return buffered
  }
}

#if INPUTIA_PAIRED_BUILD
/// 由 IMK 主线程拥有；所有认证连接与服务查询都异步，不改变 Rime 的线程归属。
final class InputiaPersonalization {
  struct View {
    let target: InputiaVoiceTarget
    let code: String
    let schemaID: String
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
  private var admissionTicket: UInt64?
  private var ticket: UInt64 = 0
  private var scheduledQuery: DispatchWorkItem?
  private var waitingQuery: DispatchWorkItem?
  private var resultsFrozen = false
  private struct CacheKey: Equatable {
    let target: InputiaVoiceTarget
    let code: String
    let schemaID: String
    let context: String
    let contextGeneration: UInt64
    let epoch: UInt64
    let server: String
    let candidates: [InputiaPersonalCandidate]
  }
  private struct CacheEntry { let key: CacheKey; let view: View; let expiry: TimeInterval }
  private var cache: [CacheEntry] = []
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
  func reset() { cache.removeAll(); context.reset(); invalidateView() }
  func invalidateView() {
    resultsFrozen = false
    admissionTicket = nil
    ticket &+= 1; scheduledQuery?.cancel(); scheduledQuery = nil
    waitingQuery?.cancel(); waitingQuery = nil; view = nil; changed?()
  }
  /// 用户开始浏览候选后保留当前顺序，正在飞行的查询不能改写数字键身份。
  func freezeResults() {
    resultsFrozen = true
    scheduledQuery?.cancel(); scheduledQuery = nil; waitingQuery?.cancel(); waitingQuery = nil
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
  func query(target: InputiaVoiceTarget, code: String, schemaID: String = "", candidates: [InputiaPersonalCandidate],
    completion: @escaping (View) -> Void) {
    guard allowed, !resultsFrozen, target.field_id != nil, candidates.count <= 64, let server else { return }
    bind(target)
    ticket &+= 1
    let version = ticket
    let contextVersion = context.generation
    scheduledQuery?.cancel(); waitingQuery?.cancel(); waitingQuery = nil
    let key = CacheKey(target: target, code: code, schemaID: schemaID, context: context.text,
      contextGeneration: contextVersion, epoch: epoch, server: server, candidates: candidates)
    let now = ProcessInfo.processInfo.systemUptime
    cache.removeAll { $0.expiry <= now }
    if let hit = cache.first(where: { $0.key == key }) {
      let restored = View(target: target, code: code, schemaID: schemaID, epoch: epoch,
        candidates: hit.view.candidates, predictions: hit.view.predictions, generation: version)
      view = restored; completion(restored); return
    }
    let work = DispatchWorkItem { [weak self] in
      guard let self, self.allowed, !self.resultsFrozen, self.ticket == version,
        self.context.generation == contextVersion, let server = self.server else { return }
      if self.queryBusy {
        self.waitingQuery?.cancel(); self.waitingQuery = self.scheduledQuery
        return
      }
      self.queryBusy = true
      let sentEpoch = self.epoch
      let command = InputiaPersonalCommand(kind: "query", target: target, learning_epoch: sentEpoch,
        input_code: code, schema_id: schemaID, context: self.context.text, context_id: target.target_id, candidates: candidates, limit: code.isEmpty ? 3 : 5)
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
        guard self.allowed, !self.resultsFrozen, InputiaPersonalContext.replyIsCurrent(ticket: self.ticket, expectedTicket: version,
          target: self.context.targetID, expectedTarget: target.target_id, context: self.context.generation,
          expectedContext: contextVersion, epoch: self.epoch, expectedEpoch: sentEpoch),
          let reply, reply.enabled, reply.epoch == sentEpoch, reply.server_instance == server,
          let result = reply.result, result.context_id == target.target_id else { return }
        let ids = result.ordered_ids ?? []
        guard InputiaPersonalContext.validOrder(ids, candidates: candidates.map { ($0.id, $0.text) }) else { return }
        let byID = Dictionary(candidates.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        let ordered = ids.compactMap { byID[$0] } + candidates.filter { !ids.contains($0.id) }
        let recalled = Self.validatedRecall(result.recalled_candidates ?? [], code: code, candidates: candidates)
        // 无共同评分标尺时保留原生首选，精确个人词紧随其后，仍保留独立学习身份。
        let merged = Array(ordered.prefix(1)) + recalled + Array(ordered.dropFirst())
        let predictions = (result.predictions ?? []).filter { !$0.text.isEmpty && $0.text.count <= 64 }.prefix(5)
        let view = View(target: target, code: code, schemaID: schemaID, epoch: sentEpoch, candidates: merged,
          predictions: Array(predictions), generation: version)
        self.cache.removeAll { $0.key == key }
        self.cache.append(CacheEntry(key: key, view: view, expiry: min(self.deadline, ProcessInfo.processInfo.systemUptime + 2)))
        if self.cache.count > 16 { self.cache.removeFirst(self.cache.count - 16) }
        self.view = view; completion(view)
      }
    }
    scheduledQuery = work
    DispatchQueue.main.async(execute: work)
  }
  static func validatedRecall(_ recalled: [InputiaPersonalCandidate], code: String,
    candidates: [InputiaPersonalCandidate]) -> [InputiaPersonalCandidate] {
    guard !code.isEmpty, code.utf8.allSatisfy({ $0 < 128 }) else { return [] }
    var texts = Set(candidates.map(\.text)); var ids = Set(candidates.map(\.id))
    return Array(recalled.filter {
      $0.id.hasPrefix("learned:") && $0.id.count > 8 && !$0.text.isEmpty && $0.text.count <= 64
        && $0.consumed_len == code.utf8.count && $0.match_type == "exact"
        && texts.insert($0.text).inserted && ids.insert($0.id).inserted
    }.prefix(5))
  }
  func admissionIsCurrent(_ admission: Admission) -> Bool {
    allowed && epoch == admission.epoch && server == admission.server && ticket == admission.ticket
      && context.targetID == admission.targetID && context.generation == admission.contextGeneration
      && view?.generation == admission.ticket && ProcessInfo.processInfo.systemUptime < admission.deadline
  }
  func admit(_ prediction: InputiaPersonalPrediction, from selected: View,
    completion: @escaping (Admission?) -> Void) {
    let isPrediction = selected.code.isEmpty && selected.predictions.contains(prediction)
    let isRecall = !selected.code.isEmpty && selected.candidates.contains {
      $0.id == prediction.id && $0.text == prediction.text && $0.id.hasPrefix("learned:")
        && $0.match_type == "exact" && $0.consumed_len == selected.code.utf8.count
    }
    guard allowed, admissionTicket == nil, isPrediction || isRecall, selected.epoch == epoch,
      selected.generation == ticket, view?.generation == ticket,
      context.targetID == selected.target.target_id,
      let server else { completion(nil); return }
    admissionTicket = ticket
    let receipt = Admission(targetID: selected.target.target_id, epoch: epoch, ticket: ticket,
      contextGeneration: context.generation, server: server,
      deadline: min(deadline, ProcessInfo.processInfo.systemUptime + 0.75))
    let command = InputiaPersonalCommand(kind: "admit", target: selected.target, learning_epoch: epoch,
      input_code: selected.code, schema_id: selected.schemaID, context: context.text, context_id: selected.target.target_id, text: prediction.text, prediction_id: prediction.id)
    InputiaVoiceInputLauncher.personalization(command, deadline: receipt.deadline, expectedServer: server) { [weak self] reply in
      guard let self else { completion(nil); return }
      if self.admissionTicket == receipt.ticket { self.admissionTicket = nil }
      guard self.admissionIsCurrent(receipt) else { completion(nil); return }
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

  func accepted(target: InputiaVoiceTarget, code: String, schemaID: String = "", text: String, explicit: Bool, rank: Int,
    previous: String? = nil, appendContext: Bool = true, operation: String = "accept", completion: @escaping () -> Void) -> Receipt? {
    guard allowed, !text.isEmpty, text.count <= 64, feedbackCount < 8, let server else { return nil }
    bind(target)
    let command = InputiaPersonalCommand(kind: "feedback", target: target, learning_epoch: epoch,
      input_code: code, schema_id: schemaID, context_id: target.target_id, event_id: UUID().uuidString, text: text,
      previous: previous ?? context.text, explicit_selection: explicit, original_rank: rank, operation: operation)
    let receipt = Receipt(command: command, server: server, epoch: epoch)
    cache.removeAll()
    if appendContext { context.append(text) }
    invalidateView(); feedbackCount += 1
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
