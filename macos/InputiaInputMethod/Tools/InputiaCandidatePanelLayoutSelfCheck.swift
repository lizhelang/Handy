import AppKit

@main
enum InputiaCandidatePanelLayoutSelfCheck {
  static func main() {
    var failures: [String] = []

    func check(_ name: String, _ condition: Bool) {
      if condition {
        print("\(name)=true")
      } else {
        print("\(name)=false")
        failures.append(name)
      }
    }

    let gap: CGFloat = 2
    let compactPanel: CGFloat = 620
    let widePanel: CGFloat = 1600
    let minFirstItemWidth: CGFloat = 44
    let minItemWidth: CGFloat = 32
    let horizontalPadding: CGFloat = 4
    let measurementSlack: CGFloat = 0

    let longCandidateRow: [CGFloat] = [336, 96, 84, 72, 68, 66, 70, 64]
    let naturalWidth = InputiaCandidateRowLayout.totalWidth(widths: longCandidateRow, itemGap: gap)
    check("longRowExceedsOldCompactWidth", naturalWidth > compactPanel)
    check("longRowFitsWidePanel", naturalWidth <= widePanel)
    check("widePanelKeepsAllEightCandidates", longCandidateRow.count == 8 && naturalWidth <= widePanel)

    let candidateMinimums = [minFirstItemWidth] + Array(repeating: minItemWidth, count: 7)
    let fallbackWidths = InputiaCandidateRowLayout.shrinkLongestFirst(
      widths: [520, 420, 360, 300, 260, 220, 180, 160],
      into: 1180,
      itemGap: gap,
      minWidths: candidateMinimums
    )
    check(
      "fallbackDoesNotOverflowScreenCap",
      InputiaCandidateRowLayout.totalWidth(widths: fallbackWidths, itemGap: gap) <= 1180
    )
    check("fallbackKeepsReadableMinimums", zip(fallbackWidths, candidateMinimums).allSatisfy { $0 >= $1 })
    check("fallbackKeepsFirstReadable", fallbackWidths.first ?? 0 >= minFirstItemWidth)
    check("fallbackAvoidsNumberOnlyItems", fallbackWidths.dropFirst().allSatisfy { $0 >= minItemWidth })
    let firstAlreadyAtMinimum = InputiaCandidateRowLayout.shrinkLongestFirst(
      widths: [44, 40, 40, 40, 40, 34, 32], into: 280, itemGap: gap,
      minWidths: [44, 32, 32, 32, 32, 32, 32]
    )
    check("fallbackShrinksOtherColumnsWhenFirstIsAtMinimum",
      firstAlreadyAtMinimum[0] == 44
        && InputiaCandidateRowLayout.totalWidth(widths: firstAlreadyAtMinimum, itemGap: gap) <= 280)

    let font = NSFont.systemFont(ofSize: 14)
    func measuredCandidateWidth(offset: Int, text: String) -> CGFloat {
      let raw = ceil(
        ("\(offset + 1) \(text)" as NSString).size(withAttributes: [.font: font]).width
          + horizontalPadding * 2
          + measurementSlack
      )
      return max(offset == 0 ? minFirstItemWidth : minItemWidth, raw)
    }

    let doublePinyinRegressionRow = [
      "现在倒反垃圾",
      "现在",
      "咸在",
      "先在",
      "先宰",
      "先载",
      "陷在",
      "见在",
    ].enumerated().map { measuredCandidateWidth(offset: $0.offset, text: $0.element) }
    check(
      "doublePinyinRegressionKeepsReadableShortItems",
      doublePinyinRegressionRow.dropFirst().allSatisfy { $0 >= minItemWidth }
    )
    check(
      "doublePinyinRegressionUsesCompactReadableRow",
      InputiaCandidateRowLayout.totalWidth(widths: doublePinyinRegressionRow, itemGap: gap) <= compactPanel
        && InputiaCandidateRowLayout.totalWidth(widths: doublePinyinRegressionRow, itemGap: gap) <= widePanel
    )
    check(
      "doublePinyinRegressionUsesOneCharacterRhythm",
      gap + horizontalPadding * 2 <= 10
    )

    var x: CGFloat = 0
    var previousMaxX: CGFloat = -gap
    var framesDoNotOverlap = true
    for width in longCandidateRow {
      if x < previousMaxX + gap - 0.5 {
        framesDoNotOverlap = false
      }
      previousMaxX = x + width
      x += width + gap
    }
    check("candidateFramesDoNotOverlap", framesDoNotOverlap)
    let fallbackFontDirectory = ProcessInfo.processInfo.environment["INPUTIA_TEST_FALLBACK_FONT_DIR"]
      .map { URL(fileURLWithPath: $0, isDirectory: true) }
      ?? URL(fileURLWithPath: CommandLine.arguments[0])
        .deletingLastPathComponent()
        .appendingPathComponent("InputiaInputMethod.app/Contents/Resources/Fonts", isDirectory: true)
    let registeredFallbackFonts = InputiaCandidateTextSupport.registerBundledFonts(
      in: fallbackFontDirectory
    )
    check("candidateTextSupportRegistersBundledFonts", registeredFallbackFonts >= 2)
    check("candidateTextSupportKeepsCommonHan", InputiaCandidateTextSupport.canDisplay("逻辑赢籯纍"))
    check("candidateTextSupportKeepsYangCandidates", InputiaCandidateTextSupport.canDisplay("洋扬痒"))
    check("candidateTextSupportKeepsSupportedNonBMPHan", InputiaCandidateTextSupport.canDisplay("𤓓𰻞"))
    check("candidateTextSupportKeepsBundledExtensionBHan", InputiaCandidateTextSupport.canDisplay("𨱍"))
    check("candidateTextSupportKeepsBundledExtensionCHan", InputiaCandidateTextSupport.canDisplay("𫗩"))
    check("candidateTextSupportRejectsPrivateUse", !InputiaCandidateTextSupport.isValidCandidateText("\u{E000}"))
    check(
      "candidateTextSupportRejectsReplacementCharacter",
      !InputiaCandidateTextSupport.isValidCandidateText("\u{FFFD}")
    )

    // 使用真实 NSView 测量和布局，回归展开面板上下切行时的宽度与列位置。
    let screenshotCandidates = [
      "你好", "妳好", "逆号", "拟好", "你", "拟", "尼",
      "泥", "呢", "妳", "妮", "腻", "逆", "倪",
      "昵", "伱", "祢", "霓", "匿", "溺", "袮",
      "鉨", "児", "睨", "铌", "旎", "埿", "阋",
      "鲵", "疑", "伲", "猊", "籾", "迡", "怩",
      "婗", "慝", "隬", "搦", "氼",
    ]
    let mixedCandidates = [
      "短", "第二列比较长的候选词", "三", "四", "五", "六", "七",
      "第一列较长", "二", "更长的第三列候选", "四", "五", "六", "七",
      "尾行", "两个",
    ]
    func snapshot(_ view: InputiaCandidateContentView, candidates: [String], fontSize: CGFloat,
      maxWidth: CGFloat, activeRow: Int, expanded: Bool = true) -> (NSSize, [NSRect]) {
      view.configure(candidates: candidates, expanded: expanded, fontSize: fontSize,
        primaryCandidateCount: 7, activeRowIndex: activeRow)
      let size = view.preferredSize(maxWidth: maxWidth)
      view.frame = NSRect(origin: .zero, size: size)
      view.layoutSubtreeIfNeeded()
      return (size, view.subviews.map(\.frame))
    }
    let content = InputiaCandidateContentView()
    content.padding = NSEdgeInsets(top: 0, left: 0, bottom: 0, right: 0)
    for (fixture, candidates) in [screenshotCandidates, mixedCandidates].enumerated() {
      let rowCount = (candidates.count + 6) / 7
      for fontSize: CGFloat in [12, 14, 22] {
        for maxWidth: CGFloat in [280, 1600] {
          let initial = snapshot(content, candidates: candidates, fontSize: fontSize,
            maxWidth: maxWidth, activeRow: 0)
          var stable = true
          var fits = true
          var widths: [CGFloat] = []
          for row in Array(0..<rowCount) + Array((0..<rowCount).reversed()) {
            let current = snapshot(content, candidates: candidates, fontSize: fontSize,
              maxWidth: maxWidth, activeRow: row)
            widths.append(current.0.width)
            stable = stable && current.0 == initial.0 && current.1 == initial.1
            fits = fits && current.0.width <= maxWidth && current.1.count == candidates.count
              && current.1.allSatisfy { $0.width > 0 && $0.height > 0 && $0.minX >= 0 && $0.maxX <= current.0.width }
          }
          let label = "fixture\(fixture)Font\(Int(fontSize))Cap\(Int(maxWidth))"
          check("expandedRowsKeepSizeAndColumns_\(label)", stable)
          check("expandedRowsFitWidth_\(label)", fits)
          print("expandedRowWidths_\(label)=\(widths.map { String(Int($0)) }.joined(separator: ","))")
        }
      }
    }
    let long = snapshot(content, candidates: mixedCandidates, fontSize: 14, maxWidth: 1600, activeRow: 0)
    let short = snapshot(content, candidates: screenshotCandidates, fontSize: 14, maxWidth: 1600, activeRow: 0)
    check("candidateChangesRecomputeWidth", long.0.width > short.0.width)
    let larger = snapshot(content, candidates: screenshotCandidates, fontSize: 22, maxWidth: 1600, activeRow: 0)
    check("fontChangesRecomputeWidth", larger.0.width > short.0.width)
    let collapsed = snapshot(content, candidates: Array(screenshotCandidates.prefix(7)),
      fontSize: 14, maxWidth: 1600, activeRow: 0, expanded: false)
    check("collapsedPanelRemainsSingleRow", collapsed.0.height == 23 && collapsed.1.count == 7)

    if failures.isEmpty {
      print("candidatePanelLayoutSelfCheckPassed=true")
    } else {
      print("candidatePanelLayoutSelfCheckPassed=false failures=\(failures.joined(separator: ","))")
      exit(1)
    }
  }
}
