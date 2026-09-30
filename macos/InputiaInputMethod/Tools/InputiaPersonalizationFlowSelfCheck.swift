import Foundation

enum InputiaPersonalizationDiagnostics {
  static func record(_ phase: String, _ reason: String, flags: Int = 0, count: Int = 0) {}
}

// 隔离的合成传输：只替换socket，不替换实际个性化状态机；不访问权限或用户输入。
struct InputiaVoiceTarget: Equatable {
  let target_id: String
  var field_id: String? = "field"
}
struct InputiaPersonalCandidate: Equatable { let id: String; let text: String; let base_rank: Int; let consumed_len: Int; var match_type: String? = nil }
struct InputiaPersonalPrediction: Equatable { let id: String; let text: String }
struct InputiaPersonalResult {
  let ordered_ids: [String]?; let predictions: [InputiaPersonalPrediction]?; let context_id: String?
  var admitted: Bool? = nil; var prediction_id: String? = nil
  var recalled_candidates: [InputiaPersonalCandidate]? = nil
}
struct InputiaPersonalReply {
  let server_instance: String; let enabled: Bool; let epoch: UInt64
  let result: InputiaPersonalResult?; let code: String?
}
struct InputiaPersonalCommand {
  let kind: String
  var target: InputiaVoiceTarget? = nil
  var learning_epoch: UInt64? = nil
  var input_code: String? = nil
  var schema_id: String? = nil
  var context: String? = nil
  var context_id: String? = nil
  var candidates: [InputiaPersonalCandidate]? = nil
  var limit: Int? = nil
  var event_id: String? = nil
  var text: String? = nil
  var previous: String? = nil
  var explicit_selection: Bool? = nil
  var original_rank: Int? = nil
  var operation: String? = nil
  var prediction_id: String? = nil
}
enum InputiaVoiceInputLauncher {
  struct Pending { let command: InputiaPersonalCommand; let completion: (InputiaPersonalReply?) -> Void }
  static var pending: [Pending] = []
  static func personalization(_ command: InputiaPersonalCommand, deadline: TimeInterval,
    expectedServer: String? = nil, completion: @escaping (InputiaPersonalReply?) -> Void) {
    pending.append(Pending(command: command, completion: completion))
  }
}
@main
struct PersonalizationFlowSelfCheck {
  static func drain(_ seconds: Double = 0.005) { RunLoop.main.run(until: Date().addingTimeInterval(seconds)) }
  // 真实个性化准入状态机 + 主机共用交付调度/等待器；只替换socket与IMK文本落点。
  static func boundaryWaitChecks() {
    let subject = InputiaPersonalization()
    subject.start()
    InputiaVoiceInputLauncher.pending.removeFirst().completion(reply())
    let target = InputiaVoiceTarget(target_id: "origin-A")
    let learned = InputiaPersonalCandidate(id: "learned:boundary", text: "倪皓", base_rank: 1,
      consumed_len: 5, match_type: "exact")
    var view: InputiaPersonalization.View?
    subject.query(target: target, code: "nihao", schemaID: "luna", candidates: []) { view = $0 }
    drain()
    InputiaVoiceInputLauncher.pending.removeFirst().completion(InputiaPersonalReply(server_instance: "service",
      enabled: true, epoch: 1, result: InputiaPersonalResult(ordered_ids: [], predictions: [],
        context_id: target.target_id, recalled_candidates: [learned]), code: nil))
    for fromGCD in [true, false] {
      for boundary in ["Return", "Tab"] {
        let waiter = InputiaPersonalBoundaryWait()
        var queue = InputiaPersonalInputDeferral<String>()
        let token = queue.begin(now: ProcessInfo.processInfo.systemUptime)!
        _ = queue.offer("x", now: ProcessInfo.processInfo.systemUptime, scopeMatches: true, boundary: false)
        var order: [String] = []
        var completed = false
        let entry = {
          subject.admit(InputiaPersonalPrediction(id: learned.id, text: learned.text), from: view!) { receipt in
            precondition(receipt != nil && subject.admissionIsCurrent(receipt!))
            order.append("admit")
            DispatchQueue.global().asyncAfter(deadline: .now() + 0.005) {
              InputiaPersonalMainDelivery.deliver {
                order.append("target-proof")
                order.append(learned.text)
                order += queue.finish(token)!
              }
            }
          }
          let rpc = InputiaVoiceInputLauncher.pending.removeFirst()
          DispatchQueue.global().asyncAfter(deadline: .now() + 0.005) {
            InputiaPersonalMainDelivery.deliver {
              rpc.completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
                result: InputiaPersonalResult(ordered_ids: nil, predictions: nil, context_id: target.target_id,
                  admitted: true, prediction_id: learned.id), code: nil))
            }
          }
          // 模拟下一个正常源的物理键。专用mode不能把它调到当前Return/Tab前。
          RunLoop.main.perform(inModes: [.default]) { order.append("later-key") }
          let outcome = waiter.wait(deadline: queue.deadline, pending: { queue.isPending }, valid: { true })
          precondition(outcome == .completed)
          precondition(order == ["admit", "target-proof", "倪皓", "x"])
          order.append(boundary) // 原事件在原栈上继续；没有insertText控制字符。
          completed = true
        }
        if fromGCD { DispatchQueue.main.async(execute: entry) }
        else { RunLoop.main.add(Timer(timeInterval: 0.001, repeats: false) { _ in entry() }, forMode: .default) }
        let deadline = Date().addingTimeInterval(1)
        while !completed && Date() < deadline { drain() }
        precondition(completed, "main GCD及RunLoop source入口均必须能收到准入/target真实交付")
        drain()
        precondition(order == ["admit", "target-proof", "倪皓", "x", boundary, "later-key"])
      }
    }
    let waiter = InputiaPersonalBoundaryWait()
    var pending = true
    InputiaPersonalMainDelivery.deliver {
      precondition(waiter.wait(deadline: ProcessInfo.processInfo.systemUptime + 1,
        pending: { true }, valid: { true }) == .reentrant)
      pending = false
    }
    precondition(waiter.wait(deadline: ProcessInfo.processInfo.systemUptime + 0.1,
      pending: { pending }, valid: { true }) == .completed)
    for _ in ["focus", "activation", "learning-epoch"] {
      var scope = true
      InputiaPersonalMainDelivery.deliver { scope = false }
      precondition(waiter.wait(deadline: ProcessInfo.processInfo.systemUptime + 0.1,
        pending: { true }, valid: { scope }) == .invalidated)
    }
    let start = ProcessInfo.processInfo.systemUptime
    precondition(waiter.wait(deadline: start + InputiaPersonalInputDeferral<String>.timeout,
      pending: { true }, valid: { true }) == .timedOut)
    let elapsed = ProcessInfo.processInfo.systemUptime - start
    precondition(elapsed >= 0.74 && elapsed < 0.9, "必须用原750ms预算并保持有界")
    subject.stop()
    print("personalBoundaryWait=true mainGCDEntry=true runLoopSourceEntry=true twoStageAdmissionDelivery=true returnTabOrder=true laterInputIsolation=true nestedWaitRejected=true targetActivationEpochCancelled=true timeout750ms=true nativeIMK=false")
  }
  static func reply(_ target: String = "origin-A", epoch: UInt64 = 1, enabled: Bool = true) -> InputiaPersonalReply {
    InputiaPersonalReply(server_instance: "service", enabled: enabled, epoch: epoch,
      result: InputiaPersonalResult(ordered_ids: ["candidate"], predictions: [], context_id: target), code: nil)
  }
  static func main() {
    let subject = InputiaPersonalization()
    subject.start()
    precondition(InputiaVoiceInputLauncher.pending.count == 1)
    InputiaVoiceInputLauncher.pending.removeFirst().completion(reply())
    let target = InputiaVoiceTarget(target_id: "origin-A")
    let candidates = [InputiaPersonalCandidate(id: "candidate", text: "你好", base_rank: 0, consumed_len: 5)]
    var installed: [String] = []
    subject.query(target: target, code: "n", candidates: candidates) { installed.append($0.code) }
    drain()
    precondition(InputiaVoiceInputLauncher.pending.count == 1)
    let old = InputiaVoiceInputLauncher.pending.removeFirst()
    subject.query(target: target, code: "ni", candidates: candidates) { installed.append($0.code) }
    drain()
    subject.query(target: target, code: "nihao", candidates: candidates) { installed.append($0.code) }
    drain()
    precondition(InputiaVoiceInputLauncher.pending.isEmpty)
    old.completion(reply())
    drain(0.02)
    precondition(installed.isEmpty)
    precondition(InputiaVoiceInputLauncher.pending.count == 1)
    let latest = InputiaVoiceInputLauncher.pending.removeFirst()
    precondition(latest.command.input_code == "nihao")
    latest.completion(reply())
    precondition(installed == ["nihao"])
    subject.query(target: target, code: "next", candidates: candidates) { installed.append($0.code) }
    drain()
    let foreign = InputiaVoiceInputLauncher.pending.removeFirst()
    subject.bind(InputiaVoiceTarget(target_id: "origin-B"))
    foreign.completion(reply())
    precondition(installed == ["nihao"] && subject.context.text.isEmpty)
    subject.bind(target)
    var predicted: InputiaPersonalization.View?
    subject.query(target: target, code: "", candidates: []) { predicted = $0 }
    drain()
    let predictionQuery = InputiaVoiceInputLauncher.pending.removeFirst()
    let prediction = InputiaPersonalPrediction(id: "next", text: "大学")
    predictionQuery.completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
      result: InputiaPersonalResult(ordered_ids: [], predictions: [prediction], context_id: target.target_id), code: nil))
    precondition(predicted != nil)
    var admission: InputiaPersonalization.Admission?
    subject.admit(prediction, from: predicted!) { admission = $0 }
    let admittedRequest = InputiaVoiceInputLauncher.pending.removeFirst()
    precondition(admittedRequest.command.kind == "admit" && admittedRequest.command.prediction_id == "next")
    admittedRequest.completion(InputiaPersonalReply(server_instance: "service", enabled: false, epoch: 2,
      result: nil, code: "policy_changed"))
    precondition(admission == nil && subject.view == nil && !subject.allowed)
    subject.stop()
    let fresh = InputiaPersonalization()
    fresh.start()
    InputiaVoiceInputLauncher.pending.removeFirst().completion(reply())
    var freshView: InputiaPersonalization.View?
    fresh.query(target: target, code: "", candidates: []) { freshView = $0 }
    drain()
    InputiaVoiceInputLauncher.pending.removeFirst().completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
      result: InputiaPersonalResult(ordered_ids: [], predictions: [prediction], context_id: target.target_id), code: nil))
    var validAdmission: InputiaPersonalization.Admission?
    fresh.admit(prediction, from: freshView!) { validAdmission = $0 }
    InputiaVoiceInputLauncher.pending.removeFirst().completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
      result: InputiaPersonalResult(ordered_ids: nil, predictions: nil, context_id: target.target_id,
        admitted: true, prediction_id: prediction.id), code: nil))
    precondition(validAdmission != nil && fresh.admissionIsCurrent(validAdmission!))
    fresh.bind(InputiaVoiceTarget(target_id: "other-field"))
    precondition(!fresh.admissionIsCurrent(validAdmission!))
    fresh.stop()
    let cached = InputiaPersonalization()
    cached.start()
    InputiaVoiceInputLauncher.pending.removeFirst().completion(reply())
    var cachedViews: [InputiaPersonalization.View] = []
    cached.query(target: target, code: "nihao", schemaID: "luna", candidates: candidates) { cachedViews.append($0) }
    drain()
    precondition(InputiaVoiceInputLauncher.pending.count == 1, "首个查询应在下个runloop发出，无120ms等待")
    let firstQuery = InputiaVoiceInputLauncher.pending.removeFirst()
    precondition(firstQuery.command.schema_id == "luna")
    let learned = InputiaPersonalCandidate(id: "learned:synthetic", text: "倪皓", base_rank: 0, consumed_len: 5, match_type: "exact")
    firstQuery.completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
      result: InputiaPersonalResult(ordered_ids: ["candidate"], predictions: [], context_id: target.target_id,
        recalled_candidates: [learned]), code: nil))
    precondition(cachedViews.last?.candidates.map(\.id) == ["candidate", "learned:synthetic"])
    cached.invalidateView()
    cached.query(target: target, code: "nihao", schemaID: "luna", candidates: candidates) { cachedViews.append($0) }
    precondition(cachedViews.count == 2 && InputiaVoiceInputLauncher.pending.isEmpty, "相同身份的短缓存应同步恢复")
    precondition(cachedViews[0].generation != cachedViews[1].generation)
    cached.query(target: target, code: "nihao", schemaID: "double_pinyin", candidates: candidates) { cachedViews.append($0) }
    drain()
    precondition(InputiaVoiceInputLauncher.pending.count == 1, "方案变更不能复用缓存")
    let frozen = InputiaVoiceInputLauncher.pending.removeFirst()
    cached.freezeResults()
    frozen.completion(reply())
    precondition(cachedViews.count == 2, "开始选择后不能异步换序")
    cached.invalidateView()
    let changedCandidates = [InputiaPersonalCandidate(id: "candidate", text: "拟好", base_rank: 0, consumed_len: 5)]
    cached.query(target: target, code: "nihao", schemaID: "luna", candidates: changedCandidates) { cachedViews.append($0) }
    drain()
    precondition(InputiaVoiceInputLauncher.pending.count == 1, "相同ID不同文本不能命中缓存")
    InputiaVoiceInputLauncher.pending.removeFirst().completion(reply())
    cached.invalidateView()
    cached.query(target: target, code: "nihao", schemaID: "luna", candidates: candidates) { cachedViews.append($0) }
    let recallView = cachedViews.last!
    var recallAdmission: InputiaPersonalization.Admission?
    cached.admit(InputiaPersonalPrediction(id: learned.id, text: learned.text), from: recallView) { recallAdmission = $0 }
    let recallRequest = InputiaVoiceInputLauncher.pending.removeFirst()
    precondition(recallRequest.command.input_code == "nihao" && recallRequest.command.schema_id == "luna")
    cached.invalidateView()
    recallRequest.completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
      result: InputiaPersonalResult(ordered_ids: nil, predictions: nil, context_id: target.target_id,
        admitted: true, prediction_id: learned.id), code: nil))
    precondition(recallAdmission == nil, "输入变化后召回准入不能继续提交")
    let malformed = InputiaPersonalCandidate(id: "rime:fake", text: "伪造", base_rank: 0, consumed_len: 5, match_type: "exact")
    let partial = InputiaPersonalCandidate(id: "learned:partial", text: "倪", base_rank: 0, consumed_len: 2, match_type: "partial")
    let duplicate = InputiaPersonalCandidate(id: "learned:duplicate", text: "你好", base_rank: 0, consumed_len: 5, match_type: "exact")
    precondition(InputiaPersonalization.validatedRecall([malformed, partial, duplicate, learned, learned],
      code: "nihao", candidates: candidates) == [learned])
    var feedbackDone = 0
    let fragmentReceipt = cached.accepted(target: target, code: "hao", schemaID: "luna", text: "皓", explicit: true,
      rank: 2, completion: { feedbackDone += 1 })!
    let derivedReceipt = cached.accepted(target: target, code: "nihao", schemaID: "luna", text: "倪皓", explicit: true,
      rank: 2, previous: "人物", appendContext: false, completion: { feedbackDone += 1 })!
    precondition(cached.context.text == "皓", "派生整词不能重复追加已上屏上下文")
    precondition(fragmentReceipt.command.event_id != derivedReceipt.command.event_id)
    precondition(derivedReceipt.command.previous == "人物" && derivedReceipt.command.schema_id == "luna")
    for request in InputiaVoiceInputLauncher.pending { request.completion(reply()) }
    InputiaVoiceInputLauncher.pending.removeAll()
    precondition(feedbackDone == 2)
    cached.undo(fragmentReceipt); cached.undo(derivedReceipt)
    precondition(InputiaVoiceInputLauncher.pending.count == 2)
    precondition(InputiaVoiceInputLauncher.pending.allSatisfy { $0.command.operation == "undo" })
    precondition(Set(InputiaVoiceInputLauncher.pending.compactMap { $0.command.event_id }).count == 2)
    for request in InputiaVoiceInputLauncher.pending { request.completion(reply()) }
    InputiaVoiceInputLauncher.pending.removeAll()
    let rejected = cached.accepted(target: target, code: "nihao", schemaID: "luna", text: "你好", explicit: true,
      rank: 0, appendContext: false, operation: "reject", completion: {})!
    precondition(rejected.command.operation == "reject" && cached.context.text.isEmpty)
    InputiaVoiceInputLauncher.pending.removeFirst().completion(reply())
    cached.stop()
    let deferredSubject = InputiaPersonalization()
    deferredSubject.start()
    InputiaVoiceInputLauncher.pending.removeFirst().completion(reply())
    var deferredView: InputiaPersonalization.View?
    deferredSubject.query(target: target, code: "nihao", schemaID: "luna", candidates: candidates) { deferredView = $0 }
    drain()
    InputiaVoiceInputLauncher.pending.removeFirst().completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
      result: InputiaPersonalResult(ordered_ids: ["candidate"], predictions: [], context_id: target.target_id,
        recalled_candidates: [learned]), code: nil))
    var inputQueue = InputiaPersonalInputDeferral<String>()
    let pendingToken = inputQueue.begin(now: 1)!
    var composing = "nihao"
    var committed = ""
    var transportAdmitted: InputiaPersonalization.Admission?
    deferredSubject.admit(InputiaPersonalPrediction(id: learned.id, text: learned.text), from: deferredView!) {
      transportAdmitted = $0
    }
    _ = inputQueue.offer("x", now: 1.01, scopeMatches: true, boundary: false)
    _ = inputQueue.offer("i", now: 1.02, scopeMatches: true, boundary: false)
    precondition(composing == "nihao" && transportAdmitted == nil)
    InputiaVoiceInputLauncher.pending.removeFirst().completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
      result: InputiaPersonalResult(ordered_ids: nil, predictions: nil, context_id: target.target_id,
        admitted: true, prediction_id: learned.id), code: nil))
    precondition(transportAdmitted != nil && deferredSubject.admissionIsCurrent(transportAdmitted!))
    // 目标服务仍在确认时，又一个普通键不会让已准入的选择过期。
    _ = inputQueue.offer("a", now: 1.03, scopeMatches: true, boundary: false)
    precondition(deferredSubject.admissionIsCurrent(transportAdmitted!))
    committed = learned.text; composing = ""
    for key in inputQueue.finish(pendingToken)! { composing += key }
    precondition(committed == "倪皓" && composing == "xia")
    // 新准入在750ms超时后取消ticket；服务迟到的成功不能吞掉下一段按键或清掉新pending。
    deferredSubject.query(target: target, code: "nihao", schemaID: "luna", candidates: candidates) { deferredView = $0 }
    let timedOut = inputQueue.begin(now: 2)!
    var lateAdmission: InputiaPersonalization.Admission?
    deferredSubject.admit(InputiaPersonalPrediction(id: learned.id, text: learned.text), from: deferredView!) { lateAdmission = $0 }
    let delayedTransport = InputiaVoiceInputLauncher.pending.removeFirst()
    _ = inputQueue.offer("x", now: 2.01, scopeMatches: true, boundary: false)
    precondition(inputQueue.expired(timedOut, now: 2.75))
    composing = "nihao"
    let timedOutKeys = inputQueue.finish(timedOut)!
    if InputiaPersonalInputDeferral<String>.allowsReplay(proofReady: false, proofDeadline: nil,
      now: 2.75, leaseMatches: true, scopeMatches: true) {
      for key in timedOutKeys { composing += key }
    }
    deferredSubject.invalidateView()
    let nextPending = inputQueue.begin(now: 3)!
    _ = inputQueue.offer("i", now: 3.01, scopeMatches: true, boundary: false)
    deferredSubject.query(target: target, code: "nihao", schemaID: "luna", candidates: candidates) { deferredView = $0 }
    var secondAdmission: InputiaPersonalization.Admission?
    deferredSubject.admit(InputiaPersonalPrediction(id: learned.id, text: learned.text), from: deferredView!) { secondAdmission = $0 }
    precondition(InputiaVoiceInputLauncher.pending.count == 1, "旧transport未完成不得阻塞取消后的新准入")
    let secondTransport = InputiaVoiceInputLauncher.pending.removeFirst()
    delayedTransport.completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
      result: InputiaPersonalResult(ordered_ids: nil, predictions: nil, context_id: target.target_id,
        admitted: true, prediction_id: learned.id), code: nil))
    precondition(lateAdmission == nil && composing == "nihao" && inputQueue.token == nextPending)
    precondition(secondAdmission == nil)
    secondTransport.completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
      result: InputiaPersonalResult(ordered_ids: nil, predictions: nil, context_id: target.target_id,
        admitted: true, prediction_id: learned.id), code: nil))
    precondition(secondAdmission != nil && deferredSubject.admissionIsCurrent(secondAdmission!))
    precondition(inputQueue.finish(nextPending) == ["i"])
    deferredSubject.stop()
    // 已发送的旧 policy 回复在撤销 ACK 之后晚到，不得重新授权候选缓存。
    let privacySubject = InputiaPersonalization()
    InputiaVoiceInputLauncher.pending.removeAll()
    privacySubject.start()
    precondition(InputiaVoiceInputLauncher.pending.count == 1)
    let oldPolicy = InputiaVoiceInputLauncher.pending.removeFirst()
    NotificationCenter.default.post(name: Notification.Name("InputiaPrivacyRevoked"), object: nil)
    oldPolicy.completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1, result: nil, code: nil))
    precondition(!privacySubject.allowed, "撤销后的旧 policy 回执不得复活租约")
    privacySubject.stop()
    let expirySubject = InputiaPersonalization()
    InputiaVoiceInputLauncher.pending.removeAll(); expirySubject.start()
    InputiaVoiceInputLauncher.pending.removeFirst().completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1, result: nil, code: nil))
    precondition(expirySubject.allowed)
    drain(2.05)
    precondition(!expirySubject.allowed, "没有新回执时租约必须在两秒内到期")
    expirySubject.stop()
    InputiaVoiceInputLauncher.pending.removeAll()
    boundaryWaitChecks()
    print("personalizationFlowSelfCheck=true coalescesLatest=true staleDiscard=true targetReset=true admitRevocation=true zeroDebounce=true scopedCache=true frozenOrder=true recallAdmission=true phraseUndo=true explicitReject=true delayedAdmitFastTyping=true delayedAdmissionTimeout=true admissionReentry=true unknownTargetNoReplay=true nativeInput=false")
  }
}
