import AppKit

struct InputiaCandidatePayload: Equatable {
  let text: String
  let annotation: String
  let source: String
  let finalScore: Int
  let originalIndex: Int

  init(
    text: String,
    annotation: String = "",
    source: String = "engine",
    finalScore: Int = 0,
    originalIndex: Int
  ) {
    self.text = text
    self.annotation = annotation
    self.source = source
    self.finalScore = finalScore
    self.originalIndex = originalIndex
  }
}

enum InputiaCandidateSection: String {
  case topSuggestion
  case mainCandidate
  case charCandidate
  case rareCandidate
}

struct InputiaCandidatePanelEntry: Equatable {
  let candidate: InputiaCandidatePayload
  let section: InputiaCandidateSection
  let label: Int?
}

struct InputiaCandidatePanelModel: Equatable {
  let topSuggestions: [InputiaCandidatePanelEntry]
  let mainCandidates: [InputiaCandidatePanelEntry]
  let charCandidates: [InputiaCandidatePanelEntry]
  let rareCandidates: [InputiaCandidatePanelEntry]
  let expanded: Bool
  let activePage: Int
  let pageSize: Int

  var selectableMainCandidates: [InputiaCandidatePanelEntry] {
    mainCandidates
  }

  static func make(
    candidates: [InputiaCandidatePayload],
    visibleCandidates: [InputiaCandidatePayload],
    expanded: Bool,
    activePage: Int,
    pageSize: Int
  ) -> InputiaCandidatePanelModel {
    let safePageSize = max(1, min(pageSize, InputiaCandidatePanelStyle.maximumMainCandidateCount))
    let allCandidates = deduplicated(candidates.isEmpty ? visibleCandidates : candidates)
    let visible = deduplicated(visibleCandidates.isEmpty ? Array(allCandidates.prefix(safePageSize)) : visibleCandidates)
    let rankedVisible = visible
      .enumerated()
      .sorted { left, right in
        compareDisplayCandidate(left.element, left.offset, right.element, right.offset)
      }
      .prefix(safePageSize)
      .enumerated()
      .map { displayIndex, pair in
        InputiaCandidatePanelEntry(
          candidate: pair.element,
          section: .mainCandidate,
          label: displayIndex + 1
        )
      }

    let mainTexts = Set(rankedVisible.map(\.candidate.text))
    let topSuggestions = allCandidates
      .enumerated()
      .filter { _, candidate in
        candidate.text.countByComposedCharacterSequences > 1
      }
      .sorted { left, right in
        compareTopSuggestion(left.element, left.offset, right.element, right.offset)
      }
      .prefix(InputiaCandidatePanelStyle.maximumTopSuggestionCount)
      .map { _, candidate in
        InputiaCandidatePanelEntry(candidate: candidate, section: .topSuggestion, label: nil)
      }
    let topTexts = Set(topSuggestions.map(\.candidate.text))

    let characterLimit = expanded
      ? InputiaCandidatePanelStyle.maximumExpandedCharCandidateCount
      : InputiaCandidatePanelStyle.maximumCollapsedCharCandidateCount
    let charCandidates = allCandidates
      .filter { candidate in
        candidate.text.countByComposedCharacterSequences == 1 && !mainTexts.contains(candidate.text)
      }
      .prefix(characterLimit)
      .map { candidate in
        InputiaCandidatePanelEntry(candidate: candidate, section: .charCandidate, label: nil)
      }

    let rareLimit = expanded ? InputiaCandidatePanelStyle.maximumRareCandidateCount : 0
    let rareCandidates = allCandidates
      .filter { candidate in
        candidate.text.countByComposedCharacterSequences > 1
          && !mainTexts.contains(candidate.text)
          && !topTexts.contains(candidate.text)
          && candidate.finalScore < 850
      }
      .prefix(rareLimit)
      .map { candidate in
        InputiaCandidatePanelEntry(candidate: candidate, section: .rareCandidate, label: nil)
      }

    return InputiaCandidatePanelModel(
      topSuggestions: Array(topSuggestions),
      mainCandidates: Array(rankedVisible),
      charCandidates: Array(charCandidates),
      rareCandidates: Array(rareCandidates),
      expanded: expanded,
      activePage: activePage,
      pageSize: safePageSize
    )
  }

  func mainCandidateOriginalIndex(forLabel label: Int) -> Int? {
    mainCandidates.first { $0.label == label }?.candidate.originalIndex
  }

  private static func deduplicated(_ candidates: [InputiaCandidatePayload]) -> [InputiaCandidatePayload] {
    var seen = Set<String>()
    var result: [InputiaCandidatePayload] = []
    for candidate in candidates where seen.insert(candidate.text).inserted {
      result.append(candidate)
    }
    return result
  }

  private static func compareDisplayCandidate(
    _ left: InputiaCandidatePayload,
    _ leftIndex: Int,
    _ right: InputiaCandidatePayload,
    _ rightIndex: Int
  ) -> Bool {
    let leftScore = displayScore(left)
    let rightScore = displayScore(right)
    if leftScore != rightScore {
      return leftScore > rightScore
    }
    return leftIndex < rightIndex
  }

  private static func compareTopSuggestion(
    _ left: InputiaCandidatePayload,
    _ leftIndex: Int,
    _ right: InputiaCandidatePayload,
    _ rightIndex: Int
  ) -> Bool {
    let leftScore = topSuggestionScore(left)
    let rightScore = topSuggestionScore(right)
    if leftScore != rightScore {
      return leftScore > rightScore
    }
    return leftIndex < rightIndex
  }

  private static func displayScore(_ candidate: InputiaCandidatePayload) -> Int {
    let length = candidate.text.countByComposedCharacterSequences
    var score = candidate.finalScore
    score += sourceBoost(candidate.source)
    score += commonPhraseBoost(candidate.text)
    if length == 1 {
      score -= 1_400
    } else if (2...4).contains(length) {
      score += 900
    } else {
      score += 240
    }
    if candidate.annotation.hasPrefix("纠错") {
      score -= 180
    }
    return score
  }

  private static func topSuggestionScore(_ candidate: InputiaCandidatePayload) -> Int {
    displayScore(candidate) + (candidate.text.countByComposedCharacterSequences > 1 ? 500 : 0)
  }

  private static func sourceBoost(_ source: String) -> Int {
    switch source {
    case "memory":
      return 1_500
    case "voice":
      return 1_200
    case "clipboard":
      return 1_000
    case "english_completion":
      return 800
    default:
      return 0
    }
  }

  private static func commonPhraseBoost(_ text: String) -> Int {
    switch text {
    case "你好", "你要", "我要", "中国", "世界":
      return 1_100
    default:
      return 0
    }
  }
}

struct InputiaCandidatePanelLayout {
  struct Cell: Equatable {
    let entry: InputiaCandidatePanelEntry
    let frame: NSRect
  }

  let size: NSSize
  let topCells: [Cell]
  let mainCells: [Cell]
  let charCells: [Cell]
  let rareCells: [Cell]
  let separatorYValues: [CGFloat]
}

enum InputiaCandidatePanelStyle {
  static let maximumCollapsedCandidateCount = 9
  static let maximumMainCandidateCount = 9
  static let maximumTopSuggestionCount = 6
  static let maximumExpandedRows = 4
  static let maximumCollapsedCharCandidateCount = 6
  static let maximumExpandedCharCandidateCount = 24
  static let maximumRareCandidateCount = 6
  static let charColumnCount = 6
  static let contentInsets = NSEdgeInsets(top: 12, left: 14, bottom: 12, right: 14)
  static let topRowHeight: CGFloat = 52
  static let mainRowHeight: CGFloat = 56
  static let charRowHeight: CGFloat = 44
  static let rareRowHeight: CGFloat = 38
  static let rowGap: CGFloat = 8
  static let columnGap: CGFloat = 10
  static let topCellWidth: CGFloat = 148
  static let mainCellMinWidth: CGFloat = 104
  static let mainCellMaxWidth: CGFloat = 174
  static let charCellWidth: CGFloat = 104
  static let rareCellWidth: CGFloat = 104
  static let minPanelWidth: CGFloat = 360
  static let maxPanelWidth: CGFloat = 1080

  static let backgroundColor = NSColor(calibratedWhite: 0.075, alpha: 0.97)
  static let borderColor = NSColor(calibratedWhite: 0.25, alpha: 0.9)
  static let separatorColor = NSColor(calibratedWhite: 0.24, alpha: 0.75)
  static let primaryTextColor = NSColor(calibratedWhite: 0.88, alpha: 1)
  static let secondaryTextColor = NSColor(calibratedWhite: 0.55, alpha: 1)
  static let mutedTextColor = NSColor(calibratedWhite: 0.42, alpha: 1)
  static let accentColor = NSColor(calibratedRed: 0.0, green: 0.72, blue: 0.45, alpha: 1)
  static let hoverColor = NSColor(calibratedWhite: 1, alpha: 0.08)

  static let topFont = NSFont.systemFont(ofSize: 24, weight: .semibold)
  static let mainFont = NSFont.systemFont(ofSize: 24, weight: .semibold)
  static let mainLabelFont = NSFont.monospacedDigitSystemFont(ofSize: 20, weight: .medium)
  static let charFont = NSFont.systemFont(ofSize: 24, weight: .medium)
  static let rareFont = NSFont.systemFont(ofSize: 20, weight: .regular)
}

enum InputiaCandidatePanelFormatter {
  static let maximumCollapsedCandidateCount = InputiaCandidatePanelStyle.maximumCollapsedCandidateCount
  static let maximumExpandedRows = InputiaCandidatePanelStyle.maximumExpandedRows
  static let usesStructuredGrid = true
  static let wrapsCandidateText = false

  static func model(
    candidates: [InputiaCandidatePayload],
    visibleCandidates: [InputiaCandidatePayload],
    expanded: Bool,
    activePage: Int = 0,
    pageSize: Int = maximumCollapsedCandidateCount
  ) -> InputiaCandidatePanelModel {
    InputiaCandidatePanelModel.make(
      candidates: candidates,
      visibleCandidates: visibleCandidates,
      expanded: expanded,
      activePage: activePage,
      pageSize: pageSize
    )
  }

  static func layout(for model: InputiaCandidatePanelModel) -> InputiaCandidatePanelLayout {
    var separatorYValues: [CGFloat] = []
    let insets = InputiaCandidatePanelStyle.contentInsets
    let mainCellWidth = preferredMainCellWidth(for: model.mainCandidates)
    let topColumns = min(
      InputiaCandidatePanelStyle.maximumTopSuggestionCount,
      max(1, model.topSuggestions.count)
    )
    let mainColumns = max(1, model.mainCandidates.count)
    let charColumns = min(InputiaCandidatePanelStyle.charColumnCount, max(1, model.charCandidates.count))
    let rareColumns = min(InputiaCandidatePanelStyle.charColumnCount, max(1, model.rareCandidates.count))
    let topWidth = model.topSuggestions.isEmpty
      ? 0
      : rowWidth(columns: topColumns, cellWidth: InputiaCandidatePanelStyle.topCellWidth)
    let mainWidth = model.mainCandidates.isEmpty
      ? 0
      : rowWidth(columns: mainColumns, cellWidth: mainCellWidth)
    let charWidth = model.charCandidates.isEmpty
      ? 0
      : rowWidth(columns: charColumns, cellWidth: InputiaCandidatePanelStyle.charCellWidth)
    let rareWidth = model.rareCandidates.isEmpty
      ? 0
      : rowWidth(columns: rareColumns, cellWidth: InputiaCandidatePanelStyle.rareCellWidth)
    let contentWidth = min(
      InputiaCandidatePanelStyle.maxPanelWidth - insets.left - insets.right,
      max(InputiaCandidatePanelStyle.minPanelWidth - insets.left - insets.right, topWidth, mainWidth, charWidth, rareWidth)
    )

    var y = insets.top
    let topCells = gridCells(
      entries: model.topSuggestions,
      x: insets.left,
      y: y,
      columns: topColumns,
      cellWidth: InputiaCandidatePanelStyle.topCellWidth,
      rowHeight: InputiaCandidatePanelStyle.topRowHeight
    )
    if !topCells.isEmpty {
      y += InputiaCandidatePanelStyle.topRowHeight
      separatorYValues.append(y + InputiaCandidatePanelStyle.rowGap / 2)
      y += InputiaCandidatePanelStyle.rowGap
    }

    let mainCells = gridCells(
      entries: model.mainCandidates,
      x: insets.left,
      y: y,
      columns: mainColumns,
      cellWidth: mainCellWidth,
      rowHeight: InputiaCandidatePanelStyle.mainRowHeight
    )
    if !mainCells.isEmpty {
      y += InputiaCandidatePanelStyle.mainRowHeight
      if !model.charCandidates.isEmpty || !model.rareCandidates.isEmpty {
        separatorYValues.append(y + InputiaCandidatePanelStyle.rowGap / 2)
        y += InputiaCandidatePanelStyle.rowGap
      }
    }

    let charRows = model.charCandidates.isEmpty
      ? 0
      : Int(ceil(Double(model.charCandidates.count) / Double(charColumns)))
    let charCells = gridCells(
      entries: model.charCandidates,
      x: insets.left,
      y: y,
      columns: charColumns,
      cellWidth: InputiaCandidatePanelStyle.charCellWidth,
      rowHeight: InputiaCandidatePanelStyle.charRowHeight
    )
    if !charCells.isEmpty {
      y += CGFloat(charRows) * InputiaCandidatePanelStyle.charRowHeight
      if !model.rareCandidates.isEmpty {
        separatorYValues.append(y + InputiaCandidatePanelStyle.rowGap / 2)
        y += InputiaCandidatePanelStyle.rowGap
      }
    }

    let rareRows = model.rareCandidates.isEmpty
      ? 0
      : Int(ceil(Double(model.rareCandidates.count) / Double(rareColumns)))
    let rareCells = gridCells(
      entries: model.rareCandidates,
      x: insets.left,
      y: y,
      columns: rareColumns,
      cellWidth: InputiaCandidatePanelStyle.rareCellWidth,
      rowHeight: InputiaCandidatePanelStyle.rareRowHeight
    )
    if !rareCells.isEmpty {
      y += CGFloat(rareRows) * InputiaCandidatePanelStyle.rareRowHeight
    }

    let width = contentWidth + insets.left + insets.right
    let height = y + insets.bottom
    return InputiaCandidatePanelLayout(
      size: NSSize(width: width, height: height),
      topCells: topCells,
      mainCells: mainCells,
      charCells: charCells,
      rareCells: rareCells,
      separatorYValues: separatorYValues
    )
  }

  static func candidateString(
    _ candidates: [String],
    expanded: Bool,
    activePage: Int = 0,
    pageSize: Int = maximumCollapsedCandidateCount
  ) -> NSAttributedString {
    let payloads = candidates.enumerated().map { index, text in
      InputiaCandidatePayload(text: text, originalIndex: index)
    }
    let model = model(
      candidates: payloads,
      visibleCandidates: Array(payloads.prefix(max(1, min(pageSize, maximumCollapsedCandidateCount)))),
      expanded: expanded,
      activePage: activePage,
      pageSize: pageSize
    )
    return NSAttributedString(string: debugString(for: model))
  }

  private static func debugString(for model: InputiaCandidatePanelModel) -> String {
    let top = model.topSuggestions.map(\.candidate.text).joined(separator: "\t")
    let main = model.mainCandidates.map { entry in
      "\(entry.label ?? 0) \(entry.candidate.text)"
    }.joined(separator: "\t")
    let chars = model.charCandidates.map(\.candidate.text).joined(separator: "\t")
    return [top, main, chars].filter { !$0.isEmpty }.joined(separator: "\n")
  }

  private static func preferredMainCellWidth(for entries: [InputiaCandidatePanelEntry]) -> CGFloat {
    guard !entries.isEmpty else {
      return InputiaCandidatePanelStyle.mainCellMinWidth
    }
    let textWidth = entries
      .map { entry in
        width(
          of: "\(entry.label ?? 0) \(entry.candidate.text)",
          font: InputiaCandidatePanelStyle.mainFont
        ) + 30
      }
      .max() ?? InputiaCandidatePanelStyle.mainCellMinWidth
    return min(
      InputiaCandidatePanelStyle.mainCellMaxWidth,
      max(InputiaCandidatePanelStyle.mainCellMinWidth, ceil(textWidth))
    )
  }

  private static func gridCells(
    entries: [InputiaCandidatePanelEntry],
    x: CGFloat,
    y: CGFloat,
    columns: Int,
    cellWidth: CGFloat,
    rowHeight: CGFloat
  ) -> [InputiaCandidatePanelLayout.Cell] {
    guard columns > 0 else {
      return []
    }
    return entries.enumerated().map { index, entry in
      let column = index % columns
      let row = index / columns
      let frame = NSRect(
        x: x + CGFloat(column) * (cellWidth + InputiaCandidatePanelStyle.columnGap),
        y: y + CGFloat(row) * rowHeight,
        width: cellWidth,
        height: rowHeight
      )
      return InputiaCandidatePanelLayout.Cell(entry: entry, frame: frame)
    }
  }

  private static func rowWidth(columns: Int, cellWidth: CGFloat) -> CGFloat {
    guard columns > 0 else {
      return 0
    }
    return CGFloat(columns) * cellWidth
      + CGFloat(max(0, columns - 1)) * InputiaCandidatePanelStyle.columnGap
  }

  private static func width(of text: String, font: NSFont) -> CGFloat {
    (text as NSString).size(withAttributes: [.font: font]).width
  }
}

final class InputiaCandidatePanelContentView: NSView {
  var model: InputiaCandidatePanelModel?
  var layout: InputiaCandidatePanelLayout?
  var selectionHandler: ((InputiaCandidatePanelEntry) -> Void)?
  private var hoveredEntry: InputiaCandidatePanelEntry?
  private var trackingArea: NSTrackingArea?

  override var isFlipped: Bool { true }

  func update(model: InputiaCandidatePanelModel, layout: InputiaCandidatePanelLayout) {
    self.model = model
    self.layout = layout
    hoveredEntry = nil
    needsDisplay = true
    updateTrackingAreas()
  }

  override func updateTrackingAreas() {
    if let trackingArea {
      removeTrackingArea(trackingArea)
    }
    let next = NSTrackingArea(
      rect: bounds,
      options: [.activeAlways, .mouseMoved, .mouseEnteredAndExited],
      owner: self,
      userInfo: nil
    )
    trackingArea = next
    addTrackingArea(next)
    super.updateTrackingAreas()
  }

  override func draw(_ dirtyRect: NSRect) {
    super.draw(dirtyRect)
    guard let layout else {
      return
    }
    for y in layout.separatorYValues {
      InputiaCandidatePanelStyle.separatorColor.setFill()
      NSRect(x: 0, y: y, width: bounds.width, height: 1).fill()
    }
    draw(cells: layout.topCells)
    draw(cells: layout.mainCells)
    draw(cells: layout.charCells)
    draw(cells: layout.rareCells)
  }

  override func mouseMoved(with event: NSEvent) {
    let point = convert(event.locationInWindow, from: nil)
    let next = entry(at: point)
    if next != hoveredEntry {
      hoveredEntry = next
      needsDisplay = true
    }
  }

  override func mouseExited(with event: NSEvent) {
    hoveredEntry = nil
    needsDisplay = true
  }

  override func mouseDown(with event: NSEvent) {
    let point = convert(event.locationInWindow, from: nil)
    guard let entry = entry(at: point) else {
      return
    }
    selectionHandler?(entry)
  }

  private func draw(cells: [InputiaCandidatePanelLayout.Cell]) {
    for cell in cells {
      draw(cell: cell)
    }
  }

  private func draw(cell: InputiaCandidatePanelLayout.Cell) {
    let entry = cell.entry
    let frame = cell.frame.insetBy(dx: 2, dy: 5)
    let isHovered = hoveredEntry == entry
    switch entry.section {
    case .topSuggestion:
      if isHovered {
        drawRoundedRect(frame, color: InputiaCandidatePanelStyle.hoverColor, radius: 8)
      }
      drawText(
        entry.candidate.text,
        in: frame.insetBy(dx: 8, dy: 9),
        font: InputiaCandidatePanelStyle.topFont,
        color: InputiaCandidatePanelStyle.primaryTextColor,
        alignment: .center
      )
    case .mainCandidate:
      let isFirst = entry.label == 1
      if isFirst {
        drawRoundedRect(frame, color: InputiaCandidatePanelStyle.accentColor, radius: 9)
      } else if isHovered {
        drawRoundedRect(frame, color: InputiaCandidatePanelStyle.hoverColor, radius: 9)
      }
      drawMainCandidate(entry, in: frame, highlighted: isFirst)
    case .charCandidate:
      if isHovered {
        drawRoundedRect(frame, color: InputiaCandidatePanelStyle.hoverColor, radius: 7)
      }
      drawText(
        entry.candidate.text,
        in: frame.insetBy(dx: 8, dy: 7),
        font: InputiaCandidatePanelStyle.charFont,
        color: InputiaCandidatePanelStyle.primaryTextColor,
        alignment: .center
      )
    case .rareCandidate:
      if isHovered {
        drawRoundedRect(frame, color: InputiaCandidatePanelStyle.hoverColor, radius: 6)
      }
      drawText(
        entry.candidate.text,
        in: frame.insetBy(dx: 8, dy: 7),
        font: InputiaCandidatePanelStyle.rareFont,
        color: InputiaCandidatePanelStyle.secondaryTextColor,
        alignment: .center
      )
    }
  }

  private func drawMainCandidate(
    _ entry: InputiaCandidatePanelEntry,
    in frame: NSRect,
    highlighted: Bool
  ) {
    let labelText = "\(entry.label ?? 0)"
    let labelFrame = NSRect(x: frame.minX + 10, y: frame.minY + 14, width: 22, height: 28)
    let textFrame = NSRect(
      x: labelFrame.maxX + 8,
      y: frame.minY + 12,
      width: max(20, frame.maxX - labelFrame.maxX - 18),
      height: 32
    )
    drawText(
      labelText,
      in: labelFrame,
      font: InputiaCandidatePanelStyle.mainLabelFont,
      color: highlighted ? .white : InputiaCandidatePanelStyle.secondaryTextColor,
      alignment: .right
    )
    drawText(
      entry.candidate.text,
      in: textFrame,
      font: InputiaCandidatePanelStyle.mainFont,
      color: highlighted ? .white : InputiaCandidatePanelStyle.primaryTextColor,
      alignment: .left
    )
  }

  private func drawRoundedRect(_ rect: NSRect, color: NSColor, radius: CGFloat) {
    color.setFill()
    NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius).fill()
  }

  private func drawText(
    _ text: String,
    in rect: NSRect,
    font: NSFont,
    color: NSColor,
    alignment: NSTextAlignment
  ) {
    let style = NSMutableParagraphStyle()
    style.lineBreakMode = .byTruncatingTail
    style.alignment = alignment
    let attributes: [NSAttributedString.Key: Any] = [
      .font: font,
      .foregroundColor: color,
      .paragraphStyle: style,
    ]
    (text as NSString).draw(in: rect, withAttributes: attributes)
  }

  private func entry(at point: NSPoint) -> InputiaCandidatePanelEntry? {
    guard let layout else {
      return nil
    }
    return (layout.topCells + layout.mainCells + layout.charCells + layout.rareCells)
      .first { $0.frame.contains(point) }?
      .entry
  }
}

final class InputiaCandidatePanel: NSPanel {
  private let backgroundView = NSView()
  private let content = InputiaCandidatePanelContentView()
  private let cursorOffset: CGFloat = 8
  private var currentModel: InputiaCandidatePanelModel?
  var selectionHandler: ((InputiaCandidatePanelEntry) -> Void)? {
    didSet {
      content.selectionHandler = selectionHandler
    }
  }

  init() {
    super.init(
      contentRect: NSRect(x: 0, y: 0, width: 360, height: 120),
      styleMask: [.nonactivatingPanel],
      backing: .buffered,
      defer: true
    )

    isReleasedWhenClosed = false
    level = NSWindow.Level(rawValue: Int(CGShieldingWindowLevel()))
    collectionBehavior = [.canJoinAllSpaces, .transient, .ignoresCycle]
    hasShadow = true
    isOpaque = false
    backgroundColor = .clear
    hidesOnDeactivate = false
    ignoresMouseEvents = false

    backgroundView.wantsLayer = true
    backgroundView.layer?.cornerRadius = 8
    backgroundView.layer?.masksToBounds = true
    backgroundView.layer?.backgroundColor = InputiaCandidatePanelStyle.backgroundColor.cgColor
    backgroundView.layer?.borderColor = InputiaCandidatePanelStyle.borderColor.cgColor
    backgroundView.layer?.borderWidth = 1
    contentView = backgroundView

    content.wantsLayer = true
    content.layer?.backgroundColor = NSColor.clear.cgColor
    backgroundView.addSubview(content)
  }

  override var canBecomeKey: Bool { false }
  override var canBecomeMain: Bool { false }

  func show(
    candidates: [String],
    near anchorRect: NSRect,
    expanded: Bool = false,
    activePage: Int = 0,
    pageSize: Int = InputiaCandidatePanelFormatter.maximumCollapsedCandidateCount
  ) {
    let payloads = candidates.enumerated().map { index, text in
      InputiaCandidatePayload(text: text, originalIndex: index)
    }
    show(
      candidates: payloads,
      visibleCandidates: payloads,
      near: anchorRect,
      expanded: expanded,
      activePage: activePage,
      pageSize: pageSize
    )
  }

  func show(
    candidates: [InputiaCandidatePayload],
    visibleCandidates: [InputiaCandidatePayload],
    near anchorRect: NSRect,
    expanded: Bool = false,
    activePage: Int = 0,
    pageSize: Int = InputiaCandidatePanelFormatter.maximumCollapsedCandidateCount
  ) {
    guard !candidates.isEmpty || !visibleCandidates.isEmpty else {
      hide()
      return
    }

    let model = InputiaCandidatePanelFormatter.model(
      candidates: candidates,
      visibleCandidates: visibleCandidates,
      expanded: expanded,
      activePage: activePage,
      pageSize: pageSize
    )
    let layout = InputiaCandidatePanelFormatter.layout(for: model)
    currentModel = model
    content.update(model: model, layout: layout)

    let anchor = Self.normalizedAnchor(anchorRect)
    var frame = NSRect(
      x: anchor.minX,
      y: anchor.minY - layout.size.height - cursorOffset,
      width: layout.size.width,
      height: layout.size.height
    )

    let visibleFrame = Self.visibleFrame(for: anchor)
    if frame.minY < visibleFrame.minY {
      frame.origin.y = anchor.maxY + cursorOffset
    }
    if frame.maxY > visibleFrame.maxY {
      frame.origin.y = visibleFrame.maxY - frame.height
    }
    if frame.maxX > visibleFrame.maxX {
      frame.origin.x = visibleFrame.maxX - frame.width
    }
    if frame.minX < visibleFrame.minX {
      frame.origin.x = visibleFrame.minX
    }

    setFrame(frame, display: true)
    backgroundView.frame = NSRect(origin: .zero, size: frame.size)
    content.frame = backgroundView.bounds
    content.updateTrackingAreas()
    orderFrontRegardless()
  }

  func mainCandidateOriginalIndex(forLabel label: Int) -> Int? {
    currentModel?.mainCandidateOriginalIndex(forLabel: label)
  }

  func hide() {
    currentModel = nil
    orderOut(nil)
  }

  private static func normalizedAnchor(_ rect: NSRect) -> NSRect {
    if rect.width > 0 || rect.height > 0 {
      return rect
    }
    return NSRect(origin: NSEvent.mouseLocation, size: NSSize(width: 1, height: 18))
  }

  private static func visibleFrame(for anchor: NSRect) -> NSRect {
    let screen = NSScreen.screens.first { $0.frame.contains(anchor.origin) } ?? NSScreen.main
    return screen?.visibleFrame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
  }
}

private extension String {
  var countByComposedCharacterSequences: Int {
    var count = 0
    enumerateSubstrings(in: startIndex..<endIndex, options: [.byComposedCharacterSequences]) { _, _, _, _ in
      count += 1
    }
    return count
  }
}
