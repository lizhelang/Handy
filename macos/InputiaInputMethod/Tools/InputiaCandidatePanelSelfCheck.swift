import Darwin
import Foundation

@main
struct InputiaCandidatePanelSelfCheck {
  static func main() {
    let candidates = (1...34).map { index in
      index == 7 ? "你来了" : "候选\(index)"
    }
    let collapsed = InputiaCandidatePanelFormatter.candidateString(
      candidates,
      expanded: false
    ).string
    let expanded = InputiaCandidatePanelFormatter.candidateString(
      candidates,
      expanded: true
    ).string

    let collapsedKeepsSingleLine = !collapsed.contains("\n")
    let collapsedContainsSeventhCandidate = collapsed.contains("7 你来了")
    let expandedKeepsRowsHorizontal = expanded.contains("1 候选1   2 候选2")
      && expanded.contains("7 你来了   8 候选8")
    let expandedBreaksOnlyAtGridRows = !expanded.contains("\n2 候选2")
      && expanded.contains("\n9 候选9")
      && expanded.contains("\n17 候选17")
      && expanded.contains("\n25 候选25")
    let expandedLineCount = expanded.split(separator: "\n", omittingEmptySubsequences: false).count
    let expandedShowsThirtyTwoCandidates = expanded.contains("32 候选32")
    let expandedCapsAtThirtyTwoCandidates = !expanded.contains("33 候选33")
    let collapsedCapsAtNineCandidates = collapsed.contains("9 候选9")
      && !collapsed.contains("10 候选10")
    let maximumCollapsedCandidateCountIsNine =
      InputiaCandidatePanelFormatter.maximumCollapsedCandidateCount == 9
    let expandedColumnCountIsEight =
      InputiaCandidatePanelFormatter.expandedColumnCount == 8
    let maximumExpandedCandidateCountIsThirtyTwo =
      InputiaCandidatePanelFormatter.maximumExpandedCandidateCount == 32
    let maximumExpandedRowsIsFour =
      InputiaCandidatePanelFormatter.maximumExpandedRows == 4

    let checks: [(String, Bool)] = [
      ("candidatePanelCollapsedKeepsSingleLine", collapsedKeepsSingleLine),
      ("candidatePanelCollapsedContainsSeventhCandidate", collapsedContainsSeventhCandidate),
      ("candidatePanelCollapsedCapsAtNineCandidates", collapsedCapsAtNineCandidates),
      ("candidatePanelExpandedKeepsRowsHorizontal", expandedKeepsRowsHorizontal),
      ("candidatePanelExpandedBreaksOnlyAtGridRows", expandedBreaksOnlyAtGridRows),
      ("candidatePanelExpandedShowsThirtyTwoCandidates", expandedShowsThirtyTwoCandidates),
      ("candidatePanelExpandedCapsAtThirtyTwoCandidates", expandedCapsAtThirtyTwoCandidates),
      ("candidatePanelMaximumCollapsedCandidateCountIsNine", maximumCollapsedCandidateCountIsNine),
      ("candidatePanelExpandedColumnCountIsEight", expandedColumnCountIsEight),
      ("candidatePanelMaximumExpandedCandidateCountIsThirtyTwo", maximumExpandedCandidateCountIsThirtyTwo),
      ("candidatePanelMaximumExpandedRowsIsFour", maximumExpandedRowsIsFour),
    ]
    let ok = checks.allSatisfy { $0.1 }
    print("candidatePanelSelfCheck=\(ok)")
    print("candidatePanelExpandedLineCount=\(expandedLineCount)")
    for (name, passed) in checks {
      print("\(name)=\(passed)")
    }
    exit(ok ? 0 : 1)
  }
}
