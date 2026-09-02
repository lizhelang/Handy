import Darwin
import Foundation

@main
struct InputiaCandidatePanelSelfCheck {
  static func main() {
    let candidates = [
      "你",
      "尼",
      "拟",
      "泥",
      "呢",
      "妮",
      "你来了",
      "逆",
      "腻",
      "第十个",
    ]
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
    let expandedBreaksLines = expanded.contains("\n2 尼")
      && expanded.contains("\n7 你来了")
      && expanded.contains("\n9 腻")
    let expandedLineCount = expanded.split(separator: "\n", omittingEmptySubsequences: false).count
    let expandedShowsNineCandidates = expandedLineCount == InputiaCandidatePanelFormatter.maximumCandidateCount
    let expandedCapsAtNineCandidates = !expanded.contains("10 第十个")
    let maximumCandidateCountIsNine = InputiaCandidatePanelFormatter.maximumCandidateCount == 9

    let checks: [(String, Bool)] = [
      ("candidatePanelCollapsedKeepsSingleLine", collapsedKeepsSingleLine),
      ("candidatePanelCollapsedContainsSeventhCandidate", collapsedContainsSeventhCandidate),
      ("candidatePanelExpandedBreaksLines", expandedBreaksLines),
      ("candidatePanelExpandedShowsNineCandidates", expandedShowsNineCandidates),
      ("candidatePanelExpandedCapsAtNineCandidates", expandedCapsAtNineCandidates),
      ("candidatePanelMaximumCandidateCountIsNine", maximumCandidateCountIsNine),
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
