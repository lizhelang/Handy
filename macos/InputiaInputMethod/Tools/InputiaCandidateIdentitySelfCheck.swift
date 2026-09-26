import Foundation

@main
struct CandidateIdentitySelfCheck {
  static func main() {
    let prior = InputiaBridgeOutcome(dictionary: ["ok": true, "consumed": true,
      "mode": "Chinese", "composing": "liming", "page": 2,
      "visible_candidates": [["id": "long-phrase", "text": "黎明"], ["id": "partial-li", "text": "黎明"]]])
    precondition(prior.candidateIDs == ["long-phrase", "partial-li"])
    precondition(prior.candidates == ["黎明", "黎明"])
    let oldABI = InputiaBridgeOutcome(dictionary: ["ok": true,
      "visible_candidates": [["text": "甲"], ["id": "B", "text": "乙"]]])
    precondition(oldABI.candidateIDs == ["", "B"])
    for failed: [String: Any]? in [nil, ["ok": false, "error": "stale expected_text"],
      ["ok": false, "composing": "", "visible_candidates": []]] {
      let result = InputiaBridgeOutcome.decodedSelection(failed, preserving: prior)
      precondition(!result.ok && result.consumed && result.commit == nil)
      precondition(result.composing == "liming" && result.page == 2 && result.mode == "Chinese")
      precondition(result.candidateIDs == prior.candidateIDs && result.candidates == prior.candidates)
    }
    let selected = InputiaBridgeOutcome.decodedSelection(["ok": true, "consumed": true,
      "commit": "黎", "mode": "Chinese", "composing": "ming", "page": 0, "visible_candidates": []], preserving: prior)
    precondition(selected.ok && selected.commit == "黎" && selected.composing == "ming")
    print("candidateIdentitySelfCheck=true duplicateTextDistinctIDs=true oldABINoGuess=true failedSelectionPreservesComposition=true nativeInput=false")
  }
}
