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
      expanded: true,
      activePage: 0,
      pageSize: 7
    )
    let layout = InputiaCandidatePanelFormatter.layout(for: model)
    let compactVisibleCandidates = [
      InputiaCandidatePayload(text: "甲乙", source: "engine", finalScore: 1_000, originalIndex: 0),
      InputiaCandidatePayload(text: "乙丙", source: "engine", finalScore: 999, originalIndex: 1),
      InputiaCandidatePayload(text: "丙丁", source: "memory", finalScore: 998, originalIndex: 2),
      InputiaCandidatePayload(text: "丁戊", source: "engine", finalScore: 997, originalIndex: 3),
      InputiaCandidatePayload(text: "戊己", source: "engine", finalScore: 996, originalIndex: 4),
      InputiaCandidatePayload(text: "己庚", source: "engine", finalScore: 995, originalIndex: 5),
      InputiaCandidatePayload(text: "庚辛", source: "engine", finalScore: 994, originalIndex: 6),
    ]
    let compactPanelCandidates = compactVisibleCandidates + [
      InputiaCandidatePayload(text: "甲", source: "engine", finalScore: 993, originalIndex: 7),
      InputiaCandidatePayload(text: "乙", source: "engine", finalScore: 992, originalIndex: 8),
      InputiaCandidatePayload(text: "丙", source: "engine", finalScore: 991, originalIndex: 9),
      InputiaCandidatePayload(text: "丁", source: "engine", finalScore: 990, originalIndex: 10),
      InputiaCandidatePayload(text: "戊", source: "engine", finalScore: 989, originalIndex: 11),
      InputiaCandidatePayload(text: "己", source: "engine", finalScore: 988, originalIndex: 12),
      InputiaCandidatePayload(text: "庚", source: "engine", finalScore: 987, originalIndex: 13),
      InputiaCandidatePayload(text: "辛", source: "engine", finalScore: 986, originalIndex: 14),
      InputiaCandidatePayload(text: "冷门", source: "engine", finalScore: 680, originalIndex: 15),
    ]
    let shortModel = InputiaCandidatePanelFormatter.model(
      candidates: compactPanelCandidates,
      visibleCandidates: compactVisibleCandidates,
      expanded: true,
      activePage: 0,
      pageSize: 7
    )
    let shortLayout = InputiaCandidatePanelFormatter.layout(for: shortModel)

    let hasStructuredGrid = InputiaCandidatePanelFormatter.usesStructuredGrid
      && !InputiaCandidatePanelFormatter.wrapsCandidateText
    let topSuggestionsArePhrases = !model.topSuggestions.isEmpty
      && model.topSuggestions.allSatisfy { $0.label == nil && $0.candidate.text.count > 1 }
    let mainCandidatesHaveLocalLabels = model.mainCandidates.map(\.label) == [1, 2, 3, 4, 5, 6, 7].map { Optional($0) }
    let commonMemoryPhraseRanksFirst = model.mainCandidates.first?.candidate.text == "你好"
      && model.mainCandidateOriginalIndex(forLabel: 1) == 2
    let singleCharactersMoveToGrid = model.charCandidates.contains { $0.candidate.text == "尼" }
      && model.charCandidates.allSatisfy { $0.candidate.text.count == 1 && $0.label == nil }
    let rareCandidatesAreSeparated = model.rareCandidates.contains {
      $0.candidate.text == "非常长的候选短语应该被截断"
    }
    let mainCellsHaveFixedHeight = layout.mainCells.allSatisfy {
      $0.frame.height == InputiaCandidatePanelStyle.mainRowHeight
    }
    let charCellsHaveFixedGrid = layout.charCells.allSatisfy {
      $0.frame.width == InputiaCandidatePanelStyle.charCellWidth
        && $0.frame.height == InputiaCandidatePanelStyle.charRowHeight
    }
    let noCandidateTextContainsNewline = (model.topSuggestions + model.mainCandidates
      + model.charCandidates + model.rareCandidates)
      .allSatisfy { !$0.candidate.text.contains("\n") }
    let panelHeightStableForTextLength = layout.size.height == shortLayout.size.height
    let panelWidthWithinLimit = layout.size.width <= InputiaCandidatePanelStyle.maxPanelWidth
    let maximumCollapsedCandidateCountIsNine =
      InputiaCandidatePanelFormatter.maximumCollapsedCandidateCount == 9
    let maximumExpandedRowsIsFour =
      InputiaCandidatePanelFormatter.maximumExpandedRows == 4

    let checks: [(String, Bool)] = [
      ("candidatePanelUsesStructuredGrid", hasStructuredGrid),
      ("candidatePanelTopSuggestionsArePhrases", topSuggestionsArePhrases),
      ("candidatePanelMainCandidatesHaveLocalLabels", mainCandidatesHaveLocalLabels),
      ("candidatePanelCommonMemoryPhraseRanksFirst", commonMemoryPhraseRanksFirst),
      ("candidatePanelSingleCharactersMoveToGrid", singleCharactersMoveToGrid),
      ("candidatePanelRareCandidatesAreSeparated", rareCandidatesAreSeparated),
      ("candidatePanelMainCellsHaveFixedHeight", mainCellsHaveFixedHeight),
      ("candidatePanelCharCellsHaveFixedGrid", charCellsHaveFixedGrid),
      ("candidatePanelNoCandidateTextContainsNewline", noCandidateTextContainsNewline),
      ("candidatePanelHeightStableForTextLength", panelHeightStableForTextLength),
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
