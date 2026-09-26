import Foundation

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
    print("personalizationSelfCheck=true boundedContext=true tokenReset=true stableIDs=true partialCode=true staleReplyRejected=true nativeUndoSourceGate=true")
  }
}
