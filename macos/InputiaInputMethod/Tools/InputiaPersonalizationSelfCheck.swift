import Foundation
import AppKit

@main
struct PersonalizationSelfCheck {
  static func main() {
    precondition(InputiaPersonalContext.hasNativeLearning(candidateID: "rime:luna:2"))
    precondition(InputiaPersonalContext.hasNativeLearning(candidateID: "rime-correction:luna:2"))
    precondition(!InputiaPersonalContext.hasNativeLearning(candidateID: "prediction:next"))
    precondition(!InputiaPersonalContext.hasNativeLearning(candidateID: "memory:term"))
    precondition(!InputiaPersonalContext.hasNativeLearning(candidateID: ""))
    var context = InputiaPersonalContext()
    context.bind("field-A")
    context.append("南京")
    context.bind("field-A")
    precondition(context.text == "南京")
    let generation = context.generation
    context.bind("field-B")
    precondition(context.text.isEmpty && context.generation != generation)
    context.append(String(repeating: "字", count: 80))
    precondition(context.text.count == 64)
    context.reset()
    precondition(context.targetID == nil && context.text.isEmpty)
    let candidates = [("id-A", "李"), ("id-B", "黎"), ("id-C", "理")]
    precondition(InputiaPersonalContext.validOrder(["id-C", "id-A"], candidates: candidates))
    precondition(!InputiaPersonalContext.validOrder(["id-C", "id-C"], candidates: candidates))
    precondition(!InputiaPersonalContext.validOrder(["forged"], candidates: candidates))
    precondition(InputiaPersonalContext.consumedCode("liming", length: 2) == "li")
    precondition(InputiaPersonalContext.consumedCode("liming", length: 0) == nil)
    precondition(InputiaPersonalContext.consumedCode("li", length: 3) == nil)
    precondition(InputiaPersonalContext.replyIsCurrent(ticket: 1, expectedTicket: 1,
      target: "A", expectedTarget: "A", context: 2, expectedContext: 2, epoch: 3, expectedEpoch: 3))
    for mismatch in 0..<4 {
      precondition(!InputiaPersonalContext.replyIsCurrent(ticket: mismatch == 0 ? 9 : 1, expectedTicket: 1,
        target: mismatch == 1 ? "B" : "A", expectedTarget: "A", context: mismatch == 2 ? 9 : 2,
        expectedContext: 2, epoch: mismatch == 3 ? 9 : 3, expectedEpoch: 3))
    }
    precondition(InputiaPersonalContext.matchesFirstPage(page: 0, expectedPage: 0, ids: ["A"], expectedIDs: ["A"]))
    precondition(!InputiaPersonalContext.matchesFirstPage(page: 1, expectedPage: 0, ids: ["A"], expectedIDs: ["A"]))
    precondition(!InputiaPersonalContext.matchesFirstPage(page: 0, expectedPage: 0, ids: ["B"], expectedIDs: ["A"]))
    precondition(!InputiaPersonalContext.retainsPersonalIdentity("promoted-rank10", available: []))
    precondition(!InputiaPersonalContext.retainsPersonalIdentity("promoted-rank10", available: ["native-rank0"]))
    var phrase = InputiaPersonalPhraseAssembly()
    precondition(phrase.confirmed(target: "A", schema: "luna", composing: "nihao", consumed: "ni",
      remaining: "hao", text: "倪", previous: "人物", start: 10, explicit: true, rank: 3) == nil)
    let joined = phrase.confirmed(target: "A", schema: "luna", composing: "hao", consumed: "hao",
      remaining: "", text: "皓", previous: "人物倪", start: 11)
    precondition(joined?.code == "nihao" && joined?.text == "倪皓" && joined?.previous == "人物")
    precondition(joined?.explicit == true && joined?.originalRank == 3)
    _ = phrase.confirmed(target: "A", schema: "luna", composing: "nihao", consumed: "ni",
      remaining: "hao", text: "你", previous: "", start: 0)
    let defaultPhrase = phrase.confirmed(target: "A", schema: "luna", composing: "hao", consumed: "hao",
      remaining: "", text: "好", previous: "你", start: 1)
    precondition(defaultPhrase?.explicit == false, "全默认空格选词不能升级为显式偏好")
    for boundary in 0..<5 {
      _ = phrase.confirmed(target: "A", schema: "luna", composing: "nihao", consumed: "ni",
        remaining: "hao", text: "倪", previous: "", start: 0)
      if boundary == 0 { phrase.reset() }
      precondition(phrase.confirmed(target: boundary == 1 ? "B" : "A", schema: boundary == 2 ? "double" : "luna",
        epoch: boundary == 4 ? 1 : 0,
        composing: "hao", consumed: "hao", remaining: "", text: "皓", previous: "", start: boundary == 3 ? 4 : 1) == nil)
    }
    _ = phrase.confirmed(target: "A", schema: "luna", composing: "nihao", consumed: "ni", remaining: "hao",
      text: "倪", previous: "", start: 0)
    precondition(phrase.confirmed(target: "A", schema: "luna", composing: "haoa", consumed: "haoa", remaining: "",
      text: "好啊", previous: "倪", start: 1) == nil, "分段后继续编辑不得学成原始整词")
    // 与主机共用的队列状态机：合成普通按键，不触碰系统注入或用户字段。
    var deferred = InputiaPersonalInputDeferral<String>()
    var composing = "nihao"
    var committed = ""
    let selection = deferred.begin(now: 10)!
    precondition(deferred.offer("x", now: 10.01, scopeMatches: true, boundary: false) == .queued)
    precondition(deferred.offer("i", now: 10.02, scopeMatches: true, boundary: false) == .queued)
    precondition(composing == "nihao", "准入前后续文字不能抢改原composition")
    committed += "倪皓"; composing = ""
    for key in deferred.finish(selection)! { composing += key }
    precondition(committed == "倪皓" && composing == "xi", "先提交已选词，再处理连续快打")
    let failed = deferred.begin(now: 11)!
    composing = "nihao"
    _ = deferred.offer("x", now: 11.01, scopeMatches: true, boundary: false)
    let failedKeys = deferred.finish(failed)!
    let recoveredProof = InputiaPersonalInputDeferral<String>.allowsReplay(proofReady: true, proofDeadline: 11.2,
      now: 11.1, leaseMatches: true, scopeMatches: true)
    if recoveredProof { for key in failedKeys { composing += key } }
    precondition(composing == "nihaox", "准入失败后重新核对原字段成功，才补齐普通文字")
    let timeout = deferred.begin(now: 12)!
    _ = deferred.offer("i", now: 12.01, scopeMatches: true, boundary: false)
    precondition(!deferred.expired(timeout, now: 12.7) && deferred.expired(timeout, now: 12.75))
    let timeoutKeys = deferred.finish(timeout)!
    let timeoutProof = InputiaPersonalInputDeferral<String>.allowsReplay(proofReady: false, proofDeadline: nil,
      now: 12.75, leaseMatches: true, scopeMatches: true)
    if timeoutProof { for key in timeoutKeys { composing += key } }
    precondition(composing == "nihaox", "超时无字段证明，不能因同代理/选区而插入缓存文字")
    for invalidProof in 0..<5 {
      precondition(!InputiaPersonalInputDeferral<String>.allowsReplay(proofReady: invalidProof != 0,
        proofDeadline: invalidProof == 1 ? nil : 20, now: invalidProof == 2 ? 20 : 19,
        leaseMatches: invalidProof != 3, scopeMatches: invalidProof != 4))
    }
    // 已排入的编辑键依原顺序处理，不能取消x再去删旧nihao。
    for boundary in ["backspace", "escape"] {
      let pending = deferred.begin(now: 13)!
      composing = ""
      _ = deferred.offer("x", now: 13.01, scopeMatches: true, boundary: false)
      precondition(deferred.offer(boundary, now: 13.02, scopeMatches: true, boundary: false) == .queued)
      for event in deferred.finish(pending)! {
        if event == "backspace" { composing.removeLast() }
        else if event == "escape" { composing = "" }
        else { composing += event }
      }
      precondition(composing.isEmpty)
    }
    let foreign = deferred.begin(now: 14)!
    _ = deferred.offer("x", now: 14.01, scopeMatches: true, boundary: false)
    precondition(deferred.offer("new-field-key", now: 14.02, scopeMatches: false, boundary: false) == .discard)
    _ = deferred.finish(foreign) // 旧字段队列不得交给新字段。
    let replacement = deferred.begin(now: 15)!
    precondition(deferred.finish(foreign) == nil && deferred.token == replacement, "过期回调不能取消下一次选择")
    for _ in 0..<InputiaPersonalInputDeferral<String>.capacity {
      precondition(deferred.offer("x", now: 15.01, scopeMatches: true, boundary: false) == .queued)
    }
    precondition(deferred.offer("overflow", now: 15.02, scopeMatches: true, boundary: false) == .flush)
    precondition(deferred.finish(replacement)?.count == InputiaPersonalInputDeferral<String>.capacity)
    let predictionSelection = deferred.begin(now: 16)!
    _ = deferred.offer("x", now: 16.01, scopeMatches: true, boundary: false)
    committed = "大学"; composing = ""
    for key in deferred.finish(predictionSelection)! { composing += key }
    precondition(committed == "大学" && composing == "x", "零码预测同样先提交再处理输入")
    // 纯Shift事件与后续A的真实类别通过共用策略，不再触发取消/flush。
    let shiftSequence: [(InputiaPersonalDeferredEventPolicy.Kind, UInt16, String?, String)] = [
      (.flagsChanged, 56, nil, "shift-down"), (.keyDown, 0, "A", "A-down"),
      (.keyUp, 0, nil, "A-up"), (.flagsChanged, 56, nil, "shift-up"), (.keyUp, 56, nil, "shift-keyup")
    ]
    let shifted = deferred.begin(now: 16.2)!
    for (kind, code, text, label) in shiftSequence {
      let eligible = InputiaPersonalDeferredEventPolicy.shouldQueue(kind: kind, keyCode: code,
        blockingModifiers: false, text: text, candidateNavigation: false)
      precondition(eligible)
      precondition(deferred.offer(label, now: 16.21, scopeMatches: true, boundary: !eligible) == .queued)
    }
    precondition(deferred.finish(shifted) == shiftSequence.map { $0.3 })
    var nativeEvents = InputiaPersonalInputDeferral<NSEvent>()
    let nativeToken = nativeEvents.begin(now: 30)!
    let eventSpec: [(NSEvent.EventType, UInt16, NSEvent.ModifierFlags, String)] = [
      (.flagsChanged, 56, [.shift], ""), (.keyDown, 0, [.shift], "A"), (.keyUp, 0, [.shift], "A"),
      (.flagsChanged, 56, [], ""), (.keyUp, 56, [], "")
    ]
    for (type, code, modifiers, characters) in eventSpec {
      let event = NSEvent.keyEvent(with: type, location: .zero, modifierFlags: modifiers, timestamp: 0,
        windowNumber: 0, context: nil, characters: characters, charactersIgnoringModifiers: characters.lowercased(),
        isARepeat: false, keyCode: code)!
      let eligible = InputiaPersonalDeferredEventPolicy.shouldQueue(event, candidateNavigation: false)
      precondition(eligible, "主机共用NSEvent入口不得把纯Shift当取消边界")
      precondition(nativeEvents.offer(event, now: 30.1, scopeMatches: true, boundary: !eligible) == .queued)
    }
    let actualEvents = nativeEvents.finish(nativeToken)!
    precondition(actualEvents.map(\.type) == eventSpec.map { $0.0 })
    precondition(actualEvents[1].characters == "A" && actualEvents[1].modifierFlags.contains(.shift))
    precondition(!InputiaPersonalDeferredEventPolicy.shouldQueue(kind: .flagsChanged, keyCode: 55,
      blockingModifiers: true, text: nil, candidateNavigation: false))
    func key(_ code: UInt16, _ text: String, modifiers: NSEvent.ModifierFlags = []) -> NSEvent {
      NSEvent.keyEvent(with: .keyDown, location: .zero, modifierFlags: modifiers, timestamp: 0,
        windowNumber: 0, context: nil, characters: text, charactersIgnoringModifiers: text,
        isARepeat: false, keyCode: code)!
    }
    let x = key(7, "x"), backspace = key(51, "\u{7f}"), escape = key(53, "\u{1b}")
    for edit in [backspace, escape, key(125, "\u{f701}"), key(116, "\u{f72c}")] {
      precondition(InputiaPersonalDeferredEventPolicy.canQueueEditing(edit, after: [x], chineseMode: true))
      precondition(!InputiaPersonalDeferredEventPolicy.canQueueEditing(edit, after: [], chineseMode: true))
      precondition(!InputiaPersonalDeferredEventPolicy.canQueueEditing(edit, after: [x, backspace], chineseMode: true))
      precondition(!InputiaPersonalDeferredEventPolicy.canQueueEditing(edit, after: [x], chineseMode: false))
      precondition(!InputiaPersonalDeferredEventPolicy.isPrintable(edit))
      precondition(!InputiaPersonalDeferredEventPolicy.canQueueEditing(edit,
        after: actualEvents + [x], chineseMode: true), "Shift可切模式，后续编辑必须保留原物理事件")
    }
    for command in [key(36, "\r"), key(48, "\t"), key(123, "\u{f702}"), key(7, "x", modifiers: [.command])] {
      precondition(!InputiaPersonalDeferredEventPolicy.shouldQueue(command, candidateNavigation: false))
      precondition(!InputiaPersonalDeferredEventPolicy.canQueueEditing(command, after: [x], chineseMode: true))
      precondition(!InputiaPersonalDeferredEventPolicy.isPrintable(command), "不能把宿主命令伪装成insertText")
    }
    // 重放中第二次召回开始后，剩余事件转入新队列；旧timeout/回调无权终结它。
    let firstRecall = deferred.begin(now: 17)!
    for key in ["2", "x", "i"] { _ = deferred.offer(key, now: 17.01, scopeMatches: true, boundary: false) }
    let replayed = deferred.finish(firstRecall)!
    var secondRecall: UInt64?
    for key in replayed {
      if key == "2" { secondRecall = deferred.begin(now: 17.1) }
      else { precondition(deferred.offer(key, now: 17.11, scopeMatches: true, boundary: false) == .queued) }
    }
    precondition(deferred.finish(firstRecall) == nil && !deferred.expired(firstRecall, now: 20))
    precondition(deferred.token == secondRecall && deferred.finish(secondRecall!) == ["x", "i"])
    print("personalizationSelfCheck=true boundedContext=true tokenReset=true stableIDs=true partialCode=true staleReplyRejected=true nativeUndoSourceGate=true phraseAssembly=true boundaryReset=true deferredInput=true deferredTimeout=true deferredTargetGate=true deferredOverflow=true deferredReentry=true replayRequiresTargetProof=true shiftedFastTyping=true nativeEventClassification=true")
  }
}
