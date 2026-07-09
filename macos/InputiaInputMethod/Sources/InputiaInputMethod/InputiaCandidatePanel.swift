import AppKit

final class InputiaCandidatePanel: NSPanel {
  private let backgroundView = NSVisualEffectView()
  private let textField = NSTextField(labelWithString: "")
  private let padding = NSEdgeInsets(top: 7, left: 10, bottom: 7, right: 10)
  private let maxPanelWidth: CGFloat = 720
  private let maxExpandedPanelWidth: CGFloat = 820
  private let cursorOffset: CGFloat = 6

  init() {
    super.init(
      contentRect: NSRect(x: 0, y: 0, width: 240, height: 36),
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
    backgroundView.layer?.cornerRadius = 7
    backgroundView.layer?.masksToBounds = true
    contentView = backgroundView

    textField.isEditable = false
    textField.isSelectable = false
    textField.drawsBackground = false
    textField.isBezeled = false
    textField.maximumNumberOfLines = 1
    textField.lineBreakMode = .byTruncatingTail
    backgroundView.addSubview(textField)
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
    guard !candidates.isEmpty else {
      hide()
      return
    }

    textField.maximumNumberOfLines = expanded ? InputiaCandidatePanelFormatter.maximumExpandedRows : 1
    textField.lineBreakMode = expanded ? .byWordWrapping : .byTruncatingTail
    let panelWidthLimit = expanded ? maxExpandedPanelWidth : maxPanelWidth
    textField.attributedStringValue = InputiaCandidatePanelFormatter.candidateString(
      candidates,
      expanded: expanded,
      activePage: activePage,
      pageSize: pageSize
    )
    let textSize = textField.attributedStringValue.boundingRect(
      with: NSSize(width: panelWidthLimit, height: .greatestFiniteMagnitude),
      options: [.usesLineFragmentOrigin, .usesFontLeading]
    ).size

    let width = min(panelWidthLimit, ceil(textSize.width) + padding.left + padding.right)
    let height = ceil(textSize.height) + padding.top + padding.bottom
    let anchor = Self.normalizedAnchor(anchorRect)
    var frame = NSRect(
      x: anchor.minX,
      y: anchor.minY - height - cursorOffset,
      width: max(width, 160),
      height: max(height, 30)
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
    textField.frame = NSRect(
      x: padding.left,
      y: padding.bottom,
      width: frame.width - padding.left - padding.right,
      height: frame.height - padding.top - padding.bottom
    )
    orderFrontRegardless()
  }

  func hide() {
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

enum InputiaCandidatePanelFormatter {
  static let maximumCollapsedCandidateCount = 9
  static let maximumExpandedRows = 4
  static let expandedCellWidth: CGFloat = 94

  static func candidateString(
    _ candidates: [String],
    expanded: Bool,
    activePage: Int = 0,
    pageSize: Int = maximumCollapsedCandidateCount
  ) -> NSAttributedString {
    let result = NSMutableAttributedString()
    let regularFont = NSFont.systemFont(ofSize: 14)
    let labelFont = NSFont.monospacedDigitSystemFont(ofSize: 12, weight: .regular)
    let paragraphStyle = NSMutableParagraphStyle()
    paragraphStyle.lineBreakMode = expanded ? .byWordWrapping : .byTruncatingTail
    paragraphStyle.tabStops = (1...maximumCollapsedCandidateCount).map { column in
      NSTextTab(textAlignment: .left, location: CGFloat(column) * expandedCellWidth)
    }
    paragraphStyle.defaultTabInterval = expandedCellWidth

    let safePageSize = max(1, min(pageSize, maximumCollapsedCandidateCount))
    let maximumCandidateCount = expanded
      ? safePageSize * maximumExpandedRows
      : maximumCollapsedCandidateCount
    let pageCount = candidatePageCount(candidates, pageSize: safePageSize)
    let activePage = max(0, min(activePage, max(0, pageCount - 1)))
    let firstVisiblePage = max(0, min(activePage, max(0, pageCount - maximumExpandedRows)))
    let activeStartIndex = activePage * safePageSize
    let visibleStartIndex = expanded ? firstVisiblePage * safePageSize : 0
    let visibleCandidates = Array(
      candidates
        .dropFirst(visibleStartIndex)
        .prefix(maximumCandidateCount)
    )

    for (visibleIndex, candidate) in visibleCandidates.enumerated() {
      let globalIndex = visibleStartIndex + visibleIndex
      let columnIndex = visibleIndex % safePageSize
      let isActiveRow = expanded && globalIndex >= activeStartIndex
        && globalIndex < activeStartIndex + safePageSize
      let labelIndex = expanded ? columnIndex + 1 : visibleIndex + 1
      let isHighlightedCandidate = expanded
        ? globalIndex == activeStartIndex
        : visibleIndex == 0

      if visibleIndex > 0 {
        let separator: String
        if expanded {
          separator = columnIndex == 0 ? "\n" : "\t"
        } else {
          separator = "   "
        }
        result.append(NSAttributedString(string: separator))
      }

      let labelAttributes: [NSAttributedString.Key: Any] = [
        .font: labelFont,
        .foregroundColor: isHighlightedCandidate ? NSColor.selectedMenuItemTextColor : NSColor.secondaryLabelColor,
        .paragraphStyle: paragraphStyle,
      ]
      let candidateAttributes: [NSAttributedString.Key: Any] = [
        .font: regularFont,
        .foregroundColor: isHighlightedCandidate ? NSColor.selectedMenuItemTextColor : NSColor.labelColor,
        .backgroundColor: isHighlightedCandidate ? NSColor.controlAccentColor : NSColor.clear,
        .paragraphStyle: paragraphStyle,
      ]

      if isActiveRow || !expanded {
        result.append(NSAttributedString(string: "\(labelIndex) ", attributes: labelAttributes))
      }
      result.append(NSAttributedString(string: candidate, attributes: candidateAttributes))
    }

    return result
  }

  private static func candidatePageCount(_ candidates: [String], pageSize: Int) -> Int {
    let safePageSize = max(1, pageSize)
    return Int(ceil(Double(candidates.count) / Double(safePageSize)))
  }
}
