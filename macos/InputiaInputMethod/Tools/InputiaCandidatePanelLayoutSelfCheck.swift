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

    if failures.isEmpty {
      print("candidatePanelLayoutSelfCheckPassed=true")
    } else {
      print("candidatePanelLayoutSelfCheckPassed=false failures=\(failures.joined(separator: ","))")
      exit(1)
    }
  }
}
