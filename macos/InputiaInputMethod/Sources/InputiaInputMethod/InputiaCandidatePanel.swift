import AppKit
import CoreText

private enum InputiaCandidatePanelDebug {
  private static let enabledPath = "/tmp/inputia-candidate-panel-debug.enabled"
  private static let logPath = "/tmp/inputia-candidate-panel.log"

  static func log(_ message: @autoclosure () -> String) {
    guard FileManager.default.fileExists(atPath: enabledPath) else {
      return
    }
    let line = "\(Date()) \(message())\n"
    guard let data = line.data(using: .utf8) else {
      return
    }
    if !FileManager.default.fileExists(atPath: logPath) {
      FileManager.default.createFile(atPath: logPath, contents: nil)
    }
    guard let file = try? FileHandle(forWritingTo: URL(fileURLWithPath: logPath)) else {
      return
    }
    defer { try? file.close() }
    _ = try? file.seekToEnd()
    _ = try? file.write(contentsOf: data)
  }
}

private enum InputiaCandidatePalette {
  static let firstHighlight = nsColor(hex: 0x2F6F73)
  static let firstHighlightHover = nsColor(hex: 0x3A8588)
  static let firstHighlightActive = nsColor(hex: 0x24575A)
  static let firstText = nsColor(hex: 0xF5FFFF)
  static let firstLabel = nsColor(hex: 0xDDEEEF)

  private static func nsColor(hex: UInt32) -> NSColor {
    NSColor(
      calibratedRed: CGFloat((hex >> 16) & 0xFF) / 255,
      green: CGFloat((hex >> 8) & 0xFF) / 255,
      blue: CGFloat(hex & 0xFF) / 255,
      alpha: 1
    )
  }
}

final class InputiaCandidatePanel: NSPanel {
  private let backgroundView = NSVisualEffectView()
  private let candidateView = InputiaCandidateContentView()
  private let padding = NSEdgeInsets(top: 0, left: 0, bottom: 0, right: 0)
  private let minPanelWidth: CGFloat = 180
  private let maxPanelWidth: CGFloat = 1600
  private let collapsedMaxPanelHeight: CGFloat = 58
  private let expandedMaxPanelHeight: CGFloat = 220
  private let cursorOffset: CGFloat = 4
  private var displaySettings = InputiaCandidateDisplaySettings.load()
  private var displaySettingsModificationDate = InputiaCandidateDisplaySettings.modificationDate()
  private static let collapsedCandidateLimit = 9
  private static let expandedCandidateLimit = 40

  init() {
    super.init(
      contentRect: NSRect(x: 0, y: 0, width: 180, height: 24),
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
    ignoresMouseEvents = true

    backgroundView.material = .hudWindow
    backgroundView.blendingMode = .behindWindow
    backgroundView.state = .active
    backgroundView.wantsLayer = true
    backgroundView.layer?.cornerRadius = 6
    backgroundView.layer?.masksToBounds = true
    contentView = backgroundView

    candidateView.padding = padding
    backgroundView.addSubview(candidateView)
    _ = InputiaCandidateTextSupport.registerBundledFonts(
      in: Bundle.main.resourceURL?.appendingPathComponent("Fonts", isDirectory: true)
    )
  }

  override var canBecomeKey: Bool { false }
  override var canBecomeMain: Bool { false }

  func show(
    candidates: [String],
    near anchorRect: NSRect,
    expanded: Bool = false,
    primaryCandidateCount: Int? = nil,
    activeRowIndex: Int = 0
  ) {
    guard !candidates.isEmpty else {
      hide()
      return
    }

    reloadDisplaySettingsIfNeeded()
    let anchor = Self.normalizedAnchor(anchorRect)
    let visibleFrame = Self.visibleFrame(for: anchor)
    let availablePanelWidth = max(minPanelWidth, min(maxPanelWidth, visibleFrame.width - 16))
    let visibleLimit = expanded ? Self.expandedCandidateLimit : Self.collapsedCandidateLimit
    candidateView.configure(
      candidates: Array(candidates.prefix(visibleLimit)),
      expanded: expanded,
      fontSize: displaySettings.fontSize,
      primaryCandidateCount: primaryCandidateCount,
      activeRowIndex: activeRowIndex
    )
    let preferredSize = candidateView.preferredSize(maxWidth: availablePanelWidth)
    let contentWidth = preferredSize.width
    let contentHeight = preferredSize.height
    let width = min(availablePanelWidth, max(minPanelWidth, contentWidth))
    let maxHeight = expanded ? expandedMaxPanelHeight : collapsedMaxPanelHeight
    let height = min(maxHeight, max(1, contentHeight))
    var frame = NSRect(
      x: anchor.minX,
      y: anchor.minY - height - cursorOffset,
      width: width,
      height: height
    )

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
    candidateView.frame = NSRect(origin: .zero, size: frame.size)
    InputiaCandidatePanelDebug.log(
      "show expanded=\(expanded) font=\(displaySettings.fontSize) available=\(Int(availablePanelWidth)) preferred=\(Int(preferredSize.width))x\(Int(preferredSize.height)) frame=\(Int(frame.width))x\(Int(frame.height)) candidates=\(Array(candidates.prefix(visibleLimit))) layout=\(candidateView.debugSummary(maxWidth: availablePanelWidth))"
    )
    orderFrontRegardless()
  }

  func hide() {
    orderOut(nil)
  }

  private func reloadDisplaySettingsIfNeeded() {
    let currentModificationDate = InputiaCandidateDisplaySettings.modificationDate()
    guard currentModificationDate != displaySettingsModificationDate else {
      return
    }
    displaySettingsModificationDate = currentModificationDate
    displaySettings = InputiaCandidateDisplaySettings.load()
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

enum InputiaCandidateTextSupport {
  private static let systemCandidateFontNames = [
    "PingFang SC",
    "Hiragino Sans GB",
    "Songti SC",
    "STSong",
    "Arial Unicode MS",
    "Maple Mono NF CN",
  ]
  private static let bundledFonts = [
    (fileName: "InputiaJigmo.ttf", fontName: "InputiaJigmo"),
    (fileName: "InputiaJigmo2.ttf", fontName: "InputiaJigmo2"),
    (fileName: "InputiaJigmo3.ttf", fontName: "InputiaJigmo3"),
  ]
  private static let registrationLock = NSLock()
  private static var registeredFontURLs = Set<URL>()

  @discardableResult
  static func registerBundledFonts(in directory: URL?) -> Int {
    guard let directory else {
      return 0
    }

    registrationLock.lock()
    defer { registrationLock.unlock() }

    var availableCount = 0
    for bundledFont in bundledFonts {
      let url = directory.appendingPathComponent(bundledFont.fileName)
      guard FileManager.default.fileExists(atPath: url.path) else {
        continue
      }
      if !registeredFontURLs.contains(url) {
        var registrationError: Unmanaged<CFError>?
        _ = CTFontManagerRegisterFontsForURL(url as CFURL, .process, &registrationError)
        registeredFontURLs.insert(url)
      }
      if NSFont(name: bundledFont.fontName, size: 14) != nil {
        availableCount += 1
      }
    }
    return availableCount
  }

  static func isValidCandidateText(_ text: String) -> Bool {
    guard !text.isEmpty else {
      return false
    }
    return text.unicodeScalars.allSatisfy { scalar in
      let value = scalar.value
      if value == 0xFFFD || scalar.properties.generalCategory == .control {
        return false
      }
      if (0xE000...0xF8FF).contains(value)
        || (0xF0000...0xFFFFD).contains(value)
        || (0x100000...0x10FFFD).contains(value)
      {
        return false
      }
      if (0xFDD0...0xFDEF).contains(value) || value & 0xFFFF == 0xFFFE || value & 0xFFFF == 0xFFFF {
        return false
      }
      return true
    }
  }

  static func canDisplay(_ text: String) -> Bool {
    guard isValidCandidateText(text) else {
      return false
    }
    return text.allSatisfy { matchingFont(for: String($0), size: 14) != nil }
  }

  static func textFont(for text: String, size: CGFloat) -> NSFont {
    matchingFont(for: text, size: size) ?? NSFont.systemFont(ofSize: size)
  }

  static func attributedText(
    _ text: String,
    size: CGFloat,
    color: NSColor,
    paragraphStyle: NSParagraphStyle
  ) -> NSAttributedString {
    let result = NSMutableAttributedString()
    for character in text {
      let segment = String(character)
      result.append(
        NSAttributedString(
          string: segment,
          attributes: [
            .font: textFont(for: segment, size: size),
            .foregroundColor: color,
            .paragraphStyle: paragraphStyle,
          ]
        )
      )
    }
    return result
  }

  private static func fontSupports(_ text: String, font: NSFont) -> Bool {
    guard !isLastResort(font) else {
      return false
    }
    let characterSet = CTFontCopyCharacterSet(font as CTFont)
    return text.unicodeScalars.allSatisfy { scalar in
      isFontNeutral(scalar) || CFCharacterSetIsLongCharacterMember(characterSet, scalar.value)
    }
  }

  private static func matchingFont(for text: String, size: CGFloat) -> NSFont? {
    if let font = systemCandidateFontNames
      .compactMap({ NSFont(name: $0, size: size) })
      .first(where: { fontSupports(text, font: $0) })
    {
      return font
    }

    let nsText = text as NSString
    if nsText.length > 0 {
      let fallback = CTFontCreateForString(
        NSFont.systemFont(ofSize: size) as CTFont,
        nsText,
        CFRange(location: 0, length: nsText.length)
      ) as NSFont
      if fontSupports(text, font: fallback) {
        return fallback
      }
    }

    return bundledFonts
      .compactMap { NSFont(name: $0.fontName, size: size) }
      .first { fontSupports(text, font: $0) }
  }

  private static func isFontNeutral(_ scalar: Unicode.Scalar) -> Bool {
    scalar.value == 0x200D
      || (0xFE00...0xFE0F).contains(scalar.value)
      || (0xE0100...0xE01EF).contains(scalar.value)
  }

  private static func isLastResort(_ font: NSFont) -> Bool {
    let name = CTFontCopyPostScriptName(font as CTFont) as String
    return name == "LastResort" || name == ".LastResort"
  }
}

private struct InputiaCandidateDisplaySettings {
  let fontSize: CGFloat

  static func load() -> Self {
    guard
      let data = try? Data(contentsOf: settingsURL()),
      let object = try? JSONSerialization.jsonObject(with: data),
      let dictionary = object as? [String: Any]
    else {
      return Self(fontSize: 14)
    }
    let rawSize = dictionary["candidate_font_size"] as? NSNumber
    return Self(fontSize: CGFloat(rawSize?.doubleValue ?? 14).clamped(to: 12...22))
  }

  static func modificationDate() -> Date? {
    try? FileManager.default.attributesOfItem(atPath: settingsURL().path)[.modificationDate] as? Date
  }

  private static func settingsURL() -> URL {
    let baseURL = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
      ?? URL(fileURLWithPath: NSTemporaryDirectory())
    return baseURL.appendingPathComponent("Inputia/settings.json")
  }
}

enum InputiaCandidateRowLayout {
  static func totalWidth(widths: [CGFloat], itemGap: CGFloat) -> CGFloat {
    widths.reduce(0, +) + CGFloat(max(0, widths.count - 1)) * itemGap
  }

  static func shrinkLongestFirst(
    widths: [CGFloat],
    into rowWidth: CGFloat,
    itemGap: CGFloat,
    minWidths: [CGFloat]
  ) -> [CGFloat] {
    guard !widths.isEmpty else {
      return []
    }
    let resolvedMinWidths = widths.indices.map { index in
      minWidths.indices.contains(index) ? minWidths[index] : (minWidths.last ?? 1)
    }
    let totalGap = CGFloat(max(0, widths.count - 1)) * itemGap
    let available = max(resolvedMinWidths.reduce(0, +), rowWidth - totalGap)
    let ideal = widths.reduce(0, +)
    guard ideal > available else {
      return widths
    }

    var fitted = widths
    var overflow = ideal - available
    let epsilon: CGFloat = 0.5
    while overflow > epsilon {
      guard let widestIndex = fitted.indices.max(by: { fitted[$0] < fitted[$1] }) else {
        break
      }
      let reducible = fitted[widestIndex] - resolvedMinWidths[widestIndex]
      guard reducible > epsilon else {
        break
      }
      let reduction = min(reducible, overflow)
      fitted[widestIndex] -= reduction
      overflow -= reduction
    }
    return fitted.indices.map { index in
      floor(max(resolvedMinWidths[index], fitted[index]))
    }
  }
}

private struct InputiaCandidateDisplayItem {
  let number: Int?
  let text: String
  let isHighlighted: Bool
}

private final class InputiaCandidateContentView: NSView {
  var padding = NSEdgeInsets(top: 2, left: 6, bottom: 2, right: 6)

  private var rows: [[InputiaCandidateDisplayItem]] = []
  private var expanded = false
  private var fontSize: CGFloat = 14
  private var primaryCandidateCount = 0
  private var activeRowIndex = 0
  private var itemViews: [InputiaCandidateItemView] = []
  private let itemGap: CGFloat = 2
  private let itemHorizontalPadding: CGFloat = 4
  private let itemMeasurementSlack: CGFloat = 0
  private let minItemWidth: CGFloat = 32
  private let minFirstItemWidth: CGFloat = 44
  private let maxTextWidth: CGFloat = 560

  override var isFlipped: Bool { true }

  func configure(
    candidates: [String],
    expanded: Bool,
    fontSize: CGFloat,
    primaryCandidateCount: Int?,
    activeRowIndex: Int
  ) {
    self.expanded = expanded
    self.fontSize = fontSize.clamped(to: 12...22)
    self.primaryCandidateCount = min(max(1, primaryCandidateCount ?? candidates.count), candidates.count)
    let rowCount = InputiaExpandedCandidateGridNavigation.rowCount(
      candidateCount: candidates.count,
      columnCount: self.primaryCandidateCount
    )
    self.activeRowIndex = expanded && rowCount > 0
      ? activeRowIndex.clamped(to: 0...(rowCount - 1))
      : 0
    self.rows = candidateRows(
      from: candidates.enumerated().map { offset, text in
        let row = self.primaryCandidateCount > 0 ? offset / self.primaryCandidateCount : 0
        let column = self.primaryCandidateCount > 0 ? offset % self.primaryCandidateCount : offset
        let isNumberedRow = !expanded || row == self.activeRowIndex
        let number = isNumberedRow && column < self.primaryCandidateCount ? column + 1 : nil
        return InputiaCandidateDisplayItem(
          number: number,
          text: text,
          isHighlighted: isNumberedRow && column == 0
        )
      }
    )
    rebuildItemViews()
    needsLayout = true
  }

  func preferredSize(maxWidth: CGFloat) -> NSSize {
    let rowHeight = self.rowHeight()
    let innerMaxWidth = max(1, maxWidth - padding.left - padding.right)
    let width: CGFloat
    if expanded, rows.count > 1 {
      let columnWidths = expandedColumnWidths()
      width = min(innerMaxWidth, InputiaCandidateRowLayout.totalWidth(widths: columnWidths, itemGap: itemGap))
    } else {
      width = rows.map { row in
        min(innerMaxWidth, rowIdealWidths(row).reduce(0, +) + CGFloat(max(0, row.count - 1)) * itemGap)
      }.max() ?? 0
    }
    return NSSize(
      width: ceil(width + padding.left + padding.right),
      height: ceil(CGFloat(rows.count) * rowHeight + padding.top + padding.bottom)
    )
  }

  func debugSummary(maxWidth: CGFloat) -> String {
    let innerMaxWidth = max(1, maxWidth - padding.left - padding.right)
    return rows.enumerated().map { rowIndex, row in
      let idealWidths = rowIdealWidths(row)
      let idealTotal = InputiaCandidateRowLayout.totalWidth(widths: idealWidths, itemGap: itemGap)
      let fittedWidths = idealTotal <= innerMaxWidth
        ? idealWidths
        : InputiaCandidateRowLayout.shrinkLongestFirst(
          widths: idealWidths,
          into: innerMaxWidth,
          itemGap: itemGap,
          minWidths: rowMinimumWidths(row)
        )
      let texts = row.map(\.text).joined(separator: "|")
      let widths = fittedWidths.map { String(Int($0)) }.joined(separator: ",")
      return "row\(rowIndex){texts=\(texts);widths=\(widths);ideal=\(Int(idealTotal));innerMax=\(Int(innerMaxWidth))}"
    }.joined(separator: " ")
  }

  override func layout() {
    super.layout()
    guard !rows.isEmpty else {
      return
    }

    let rowHeight = self.rowHeight()
    let innerWidth = max(1, bounds.width - padding.left - padding.right)
    let sharedColumnWidths = expanded && rows.count > 1 ? fittedExpandedColumnWidths(into: innerWidth) : nil
    var viewIndex = 0
    for (rowIndex, row) in rows.enumerated() {
      let rowY = padding.top + CGFloat(rowIndex) * rowHeight
      layout(
        row: row,
        in: NSRect(x: padding.left, y: rowY, width: innerWidth, height: rowHeight),
        sharedColumnWidths: sharedColumnWidths,
        viewIndex: &viewIndex
      )
    }
  }

  private func candidateRows(
    from indexed: [InputiaCandidateDisplayItem]
  ) -> [[InputiaCandidateDisplayItem]] {
    guard expanded, indexed.count > primaryCandidateCount, primaryCandidateCount > 0 else {
      return [indexed]
    }
    let columnCount = primaryCandidateCount.clamped(to: 1...9)
    var rows = [Array(indexed.prefix(columnCount))]
    var remaining = Array(indexed.dropFirst(columnCount))
    while !remaining.isEmpty {
      rows.append(Array(remaining.prefix(columnCount)))
      remaining = Array(remaining.dropFirst(columnCount))
    }
    return rows
  }

  private func layout(
    row: [InputiaCandidateDisplayItem],
    in rowRect: NSRect,
    sharedColumnWidths: [CGFloat]?,
    viewIndex: inout Int
  ) {
    let idealWidths = rowIdealWidths(row)
    let baseWidths = sharedColumnWidths.map { Array($0.prefix(row.count)) } ?? idealWidths
    let idealTotalWidth = InputiaCandidateRowLayout.totalWidth(widths: baseWidths, itemGap: itemGap)
    let itemWidths = idealTotalWidth <= rowRect.width
      ? baseWidths
      : InputiaCandidateRowLayout.shrinkLongestFirst(
        widths: baseWidths,
        into: rowRect.width,
        itemGap: itemGap,
        minWidths: rowMinimumWidths(row)
      )
    var x = rowRect.minX
    for index in row.indices {
      guard viewIndex < itemViews.count else {
        return
      }
      let itemWidth = itemWidths[index]
      itemViews[viewIndex].frame = NSRect(
        x: x,
        y: rowRect.minY,
        width: itemWidth,
        height: rowRect.height
      ).integral
      viewIndex += 1
      x += itemWidth + itemGap
    }
  }

  private func rowIdealWidths(_ row: [InputiaCandidateDisplayItem]) -> [CGFloat] {
    row.map { item in
      let value = attributedCandidateText(for: item, highlighted: item.isHighlighted)
      let measuredWidth = ceil(min(value.size().width, maxTextWidth) + itemHorizontalPadding * 2 + itemMeasurementSlack)
      return max(minimumWidth(for: item), measuredWidth)
    }
  }

  private func rowMinimumWidths(_ row: [InputiaCandidateDisplayItem]) -> [CGFloat] {
    row.map { item in
      minimumWidth(for: item)
    }
  }

  private func expandedColumnWidths() -> [CGFloat] {
    let columnCount = rows.map(\.count).max() ?? 0
    guard columnCount > 0 else {
      return []
    }
    var widths = Array(repeating: minItemWidth, count: columnCount)
    for row in rows {
      let idealWidths = rowIdealWidths(row)
      for index in idealWidths.indices {
        widths[index] = max(widths[index], idealWidths[index])
      }
    }
    return widths
  }

  private func expandedColumnMinimumWidths() -> [CGFloat] {
    let columnCount = rows.map(\.count).max() ?? 0
    guard columnCount > 0 else {
      return []
    }
    var widths = Array(repeating: minItemWidth, count: columnCount)
    for row in rows {
      let minimumWidths = rowMinimumWidths(row)
      for index in minimumWidths.indices {
        widths[index] = max(widths[index], minimumWidths[index])
      }
    }
    return widths
  }

  private func fittedExpandedColumnWidths(into innerWidth: CGFloat) -> [CGFloat] {
    let widths = expandedColumnWidths()
    let idealTotal = InputiaCandidateRowLayout.totalWidth(widths: widths, itemGap: itemGap)
    guard idealTotal > innerWidth else {
      return widths
    }
    return InputiaCandidateRowLayout.shrinkLongestFirst(
      widths: widths,
      into: innerWidth,
      itemGap: itemGap,
      minWidths: expandedColumnMinimumWidths()
    )
  }

  private func minimumWidth(for item: InputiaCandidateDisplayItem) -> CGFloat {
    item.isHighlighted ? minFirstItemWidth : minItemWidth
  }

  private func rowHeight() -> CGFloat {
    ceil(fontSize + 9)
  }

  private func textFont() -> NSFont {
    .systemFont(ofSize: fontSize)
  }

  private func rebuildItemViews() {
    itemViews.forEach { $0.removeFromSuperview() }
    itemViews = rows.flatMap { $0 }.map { item in
      let view = InputiaCandidateItemView(horizontalPadding: itemHorizontalPadding)
      view.configure(
        text: attributedCandidateText(for: item, highlighted: item.isHighlighted),
        highlighted: item.isHighlighted
      )
      addSubview(view)
      return view
    }
  }

  private func attributedCandidateText(
    for item: InputiaCandidateDisplayItem,
    highlighted: Bool
  ) -> NSAttributedString {
    let value = NSMutableAttributedString()
    if let number = item.number {
      value.append(
        NSAttributedString(
          string: "\(number) ",
          attributes: [
            .font: textFont(),
            .foregroundColor: highlighted ? InputiaCandidatePalette.firstLabel : NSColor.secondaryLabelColor,
            .paragraphStyle: clippingParagraphStyle(),
          ]
        )
      )
    }
    value.append(
      InputiaCandidateTextSupport.attributedText(
        item.text,
        size: fontSize,
        color: highlighted ? InputiaCandidatePalette.firstText : NSColor.labelColor,
        paragraphStyle: clippingParagraphStyle()
      )
    )
    return value
  }

  private func clippingParagraphStyle() -> NSParagraphStyle {
    let style = NSMutableParagraphStyle()
    style.lineBreakMode = .byTruncatingTail
    return style
  }
}

private final class InputiaCandidateItemView: NSView {
  private var text = NSAttributedString(string: "")
  private let horizontalPadding: CGFloat

  override var isFlipped: Bool { true }

  init(horizontalPadding: CGFloat) {
    self.horizontalPadding = horizontalPadding
    super.init(frame: .zero)
    wantsLayer = true
    layer?.cornerRadius = 6
    layer?.masksToBounds = true
  }

  required init?(coder: NSCoder) {
    nil
  }

  func configure(text: NSAttributedString, highlighted: Bool) {
    self.text = text
    layer?.backgroundColor = highlighted ? InputiaCandidatePalette.firstHighlight.cgColor : NSColor.clear.cgColor
    needsDisplay = true
  }

  override func draw(_ dirtyRect: NSRect) {
    super.draw(dirtyRect)
    let textSize = text.size()
    let origin = NSPoint(
      x: horizontalPadding,
      y: floor((bounds.height - textSize.height) / 2)
    )
    text.draw(at: origin)
  }
}

private extension Comparable {
  func clamped(to limits: ClosedRange<Self>) -> Self {
    min(max(self, limits.lowerBound), limits.upperBound)
  }
}
