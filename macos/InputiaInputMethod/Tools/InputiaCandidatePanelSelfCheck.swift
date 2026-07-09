import Darwin
import Foundation

@main
struct InputiaCandidatePanelSelfCheck {
  static func main() {
    let candidates = [
      "甲", "乙", "丙", "丁", "戊", "己", "庚",
      "辛", "壬", "癸", "子", "丑", "寅", "卯",
      "辰", "巳", "午", "未", "申", "酉", "戌",
      "亥", "天", "地", "玄", "黄", "宇", "宙",
      "洪", "荒", "日", "月", "盈", "昃",
    ]
    let collapsed = InputiaCandidatePanelFormatter.candidateString(
      candidates,
      expanded: false
    ).string
    let expanded = InputiaCandidatePanelFormatter.candidateString(
      candidates,
      expanded: true,
      activePage: 0,
      pageSize: 7
    ).string

    let collapsedKeepsSingleLine = !collapsed.contains("\n")
    let collapsedContainsSeventhCandidate = collapsed.contains("7 庚")
    let expandedKeepsRowsHorizontal = expanded.contains("1 甲\t2 乙")
      && expanded.contains("6 己\t7 庚")
    let expandedBreaksOnlyAtPageRows = !expanded.contains("\n2 乙")
      && expanded.contains("\n辛\t壬")
      && expanded.contains("\n辰\t巳")
      && expanded.contains("\n亥\t天")
    let expandedLineCount = expanded.split(separator: "\n", omittingEmptySubsequences: false).count
    let expandedShowsFourPages = expanded.contains("宙")
    let expandedCapsAtFourPages = !expanded.contains("洪")
    let expandedDoesNotUseGlobalLabels = !expanded.contains("8 辛")
      && !expanded.contains("14 卯")
      && !expanded.contains("22 亥")
      && !expanded.contains("28 宙")
    let secondPageExpanded = InputiaCandidatePanelFormatter.candidateString(
      candidates,
      expanded: true,
      activePage: 1,
      pageSize: 7
    ).string
    let secondPageUsesLocalLabels = secondPageExpanded.contains("1 辛\t2 壬")
      && secondPageExpanded.contains("6 寅\t7 卯")
      && !secondPageExpanded.contains("8 辛")
      && !secondPageExpanded.contains("14 卯")
    let collapsedCapsAtNineCandidates = collapsed.contains("9 壬")
      && !collapsed.contains("10 癸")
    let maximumCollapsedCandidateCountIsNine =
      InputiaCandidatePanelFormatter.maximumCollapsedCandidateCount == 9
    let maximumExpandedRowsIsFour =
      InputiaCandidatePanelFormatter.maximumExpandedRows == 4

    let checks: [(String, Bool)] = [
      ("candidatePanelCollapsedKeepsSingleLine", collapsedKeepsSingleLine),
      ("candidatePanelCollapsedContainsSeventhCandidate", collapsedContainsSeventhCandidate),
      ("candidatePanelCollapsedCapsAtNineCandidates", collapsedCapsAtNineCandidates),
      ("candidatePanelExpandedKeepsRowsHorizontal", expandedKeepsRowsHorizontal),
      ("candidatePanelExpandedBreaksOnlyAtPageRows", expandedBreaksOnlyAtPageRows),
      ("candidatePanelExpandedShowsFourPages", expandedShowsFourPages),
      ("candidatePanelExpandedCapsAtFourPages", expandedCapsAtFourPages),
      ("candidatePanelExpandedDoesNotUseGlobalLabels", expandedDoesNotUseGlobalLabels),
      ("candidatePanelSecondPageUsesLocalLabels", secondPageUsesLocalLabels),
      ("candidatePanelMaximumCollapsedCandidateCountIsNine", maximumCollapsedCandidateCountIsNine),
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
