import Darwin
import Foundation

@main
struct InputiaCandidatePanelSelfCheck {
  static func main() {
    let visibleCandidates = [
      InputiaCandidatePayload(text: "你好看", source: "engine", finalScore: 1_000, originalIndex: 0),
      InputiaCandidatePayload(text: "你会", source: "engine", finalScore: 999, originalIndex: 1),
      InputiaCandidatePayload(text: "你好", source: "memory", finalScore: 998, originalIndex: 2),
      InputiaCandidatePayload(text: "拟好", source: "engine", finalScore: 997, originalIndex: 3),
      InputiaCandidatePayload(text: "妳好", source: "engine", finalScore: 996, originalIndex: 4),
      InputiaCandidatePayload(text: "逆号", source: "engine", finalScore: 995, originalIndex: 5),
      InputiaCandidatePayload(text: "你要", source: "engine", finalScore: 994, originalIndex: 6),
    ]
    let panelCandidates = visibleCandidates + [
      InputiaCandidatePayload(text: "尼", source: "engine", finalScore: 993, originalIndex: 7),
      InputiaCandidatePayload(text: "泥", source: "engine", finalScore: 992, originalIndex: 8),
      InputiaCandidatePayload(text: "呢", source: "engine", finalScore: 991, originalIndex: 9),
      InputiaCandidatePayload(text: "你", source: "engine", finalScore: 990, originalIndex: 10),
      InputiaCandidatePayload(text: "妳", source: "engine", finalScore: 989, originalIndex: 11),
      InputiaCandidatePayload(text: "拟", source: "engine", finalScore: 988, originalIndex: 12),
      InputiaCandidatePayload(text: "旎", source: "engine", finalScore: 700, originalIndex: 13),
      InputiaCandidatePayload(text: "鲵", source: "engine", finalScore: 690, originalIndex: 14),
      InputiaCandidatePayload(text: "非常长的候选短语应该被截断", source: "engine", finalScore: 680, originalIndex: 15),
    ]

    let model = InputiaCandidatePanelFormatter.model(
      candidates: panelCandidates,
      visibleCandidates: visibleCandidates,
      expanded: false,
      activePage: 0,
      pageSize: 7
    )
    let layout = InputiaCandidatePanelFormatter.layout(for: model)
    let expandedModel = InputiaCandidatePanelFormatter.model(
      candidates: panelCandidates,
      visibleCandidates: visibleCandidates,
      expanded: true,
      activePage: 0,
      pageSize: 7
    )
    let expandedLayout = InputiaCandidatePanelFormatter.layout(for: expandedModel)

    let hasStructuredGrid = InputiaCandidatePanelFormatter.usesStructuredGrid
      && !InputiaCandidatePanelFormatter.wrapsCandidateText
    let singleRowOnly = model.topSuggestions.isEmpty
      && model.charCandidates.isEmpty
      && model.rareCandidates.isEmpty
      && expandedModel.topSuggestions.isEmpty
      && expandedModel.charCandidates.isEmpty
      && expandedModel.rareCandidates.isEmpty
    let mainCandidatesHaveLocalLabels = model.mainCandidates.map(\.label) == [1, 2, 3, 4, 5, 6, 7].map { Optional($0) }
    let commonMemoryPhraseRanksFirst = model.mainCandidates.first?.candidate.text == "你好"
      && model.mainCandidateOriginalIndex(forLabel: 1) == 2
    let mainCellsHaveFixedHeight = layout.mainCells.allSatisfy {
      $0.frame.height == InputiaCandidatePanelStyle.mainRowHeight
    }
    let noCandidateTextContainsNewline = model.mainCandidates
      .allSatisfy { !$0.candidate.text.contains("\n") }
    let collapsedIsSingleRow = layout.topCells.isEmpty
      && layout.charCells.isEmpty
      && layout.rareCells.isEmpty
      && layout.separatorYValues.isEmpty
      && !layout.mainCells.isEmpty
    let expandedAlsoSingleCandidateRow = expandedLayout.topCells.isEmpty
      && expandedLayout.charCells.isEmpty
      && expandedLayout.rareCells.isEmpty
    let panelWidthWithinLimit = layout.size.width <= InputiaCandidatePanelStyle.maxPanelWidth
    let maximumCollapsedCandidateCountIsNine =
      InputiaCandidatePanelFormatter.maximumCollapsedCandidateCount == 9
    let maximumExpandedRowsIsFour =
      InputiaCandidatePanelFormatter.maximumExpandedRows == 4

    let checks: [(String, Bool)] = [
      ("candidatePanelUsesStructuredGrid", hasStructuredGrid),
      ("candidatePanelShowsSingleRowOnly", singleRowOnly),
      ("candidatePanelMainCandidatesHaveLocalLabels", mainCandidatesHaveLocalLabels),
      ("candidatePanelCommonMemoryPhraseRanksFirst", commonMemoryPhraseRanksFirst),
      ("candidatePanelMainCellsHaveFixedHeight", mainCellsHaveFixedHeight),
      ("candidatePanelNoCandidateTextContainsNewline", noCandidateTextContainsNewline),
      ("candidatePanelCollapsedIsSingleRow", collapsedIsSingleRow),
      ("candidatePanelExpandedAlsoSingleCandidateRow", expandedAlsoSingleCandidateRow),
      ("candidatePanelWidthWithinLimit", panelWidthWithinLimit),
      ("candidatePanelMaximumCollapsedCandidateCountIsNine", maximumCollapsedCandidateCountIsNine),
      ("candidatePanelMaximumExpandedRowsIsFour", maximumExpandedRowsIsFour),
    ]
    let ok = checks.allSatisfy { $0.1 }
    print("candidatePanelSelfCheck=\(ok)")
    print("candidatePanelTopSuggestionCount=\(model.topSuggestions.count)")
    print("candidatePanelMainCandidateCount=\(model.mainCandidates.count)")
    print("candidatePanelCharCandidateCount=\(model.charCandidates.count)")
    print("candidatePanelRareCandidateCount=\(model.rareCandidates.count)")
    print("candidatePanelHeight=\(Int(layout.size.height))")
    print("candidatePanelWidth=\(Int(layout.size.width))")
    for (name, passed) in checks {
      print("\(name)=\(passed)")
    }
    exit(ok ? 0 : 1)
  }
}
