#!/usr/bin/env python3
"""无窗口：真实候选 NSView 的更新与真实呈现方法的窗口调用顺序。"""
import argparse
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--content-source", type=Path)
args = parser.parse_args()
source = (root / "Sources/InputiaInputMethod/InputiaCandidatePanel.swift").read_text()
content_source = args.content_source.read_text() if args.content_source else source


def declaration(text, marker):
    start = text.index(marker)
    brace = text.index("{", start)
    end, depth = brace + 1, 1
    while depth:
        depth += (text[end] == "{") - (text[end] == "}")
        end += 1
    return text[start:end]


content = "\n".join(declaration(content_source, marker) for marker in [
    "private enum InputiaCandidatePalette", "enum InputiaCandidateTextSupport",
    "enum InputiaCandidateRowLayout", "private struct InputiaCandidateDisplayItem",
    "final class InputiaCandidateContentView", "private final class InputiaCandidateItemView",
    "private extension Comparable",
])
content_test = r'''
@main struct ContentCheck {
  static func main() {
    let view = InputiaCandidateContentView()
    view.frame = NSRect(x: 0, y: 0, width: 620, height: 120)
    let words = ["公开", "候选", "显示", "检查"]
    func configure(_ candidates: [String] = words, expanded: Bool = false, font: CGFloat = 14,
      primary: Int? = nil, active: Int = 0) {
      view.configure(candidates: candidates, expanded: expanded, fontSize: font,
        primaryCandidateCount: primary, activeRowIndex: active)
      view.layoutSubtreeIfNeeded()
    }
    func ids() -> [ObjectIdentifier] { view.subviews.map(ObjectIdentifier.init) }
    configure(); var previous = view.subviews
    configure()
    precondition(previous.map(ObjectIdentifier.init) == ids(), "相同完整显示状态必须复用行视图")
    precondition(view.subviews.allSatisfy { $0.frame.width > 0 && $0.frame.height > 0 })
    print("identicalRenderReusesRows=true noZeroFrameReplacement=true")
    for change in 0..<4 {
      previous = view.subviews
      switch change {
      case 0: configure(Array(words.reversed()))
      case 1: configure(font: 20)
      case 2: configure(expanded: true, primary: 2)
      default: configure(expanded: true, primary: 2, active: 1)
      }
      precondition(Set(previous.map(ObjectIdentifier.init)).isDisjoint(with: Set(ids())),
        "候选顺序/字体/列布局/活动行变化必须更新显示")
    }
    previous = view.subviews
    configure(expanded: true, primary: 3, active: 1)
    precondition(Set(previous.map(ObjectIdentifier.init)).isDisjoint(with: Set(ids())))
    print("candidateFontExpansionNumberingAndActiveRowInvalidate=true")
    configure(); previous = view.subviews
    view.padding.left += 12
    view.layoutSubtreeIfNeeded()
    precondition(previous.map(ObjectIdentifier.init) == ids() && view.subviews[0].frame.minX == view.padding.left)
    print("paddingRelayoutDoesNotRebuildText=true")
    view.configureStatus("", fontSize: 14); view.layoutSubtreeIfNeeded()
    previous = view.subviews
    view.configureStatus("", fontSize: 14); view.layoutSubtreeIfNeeded()
    precondition(previous.map(ObjectIdentifier.init) == ids())
    precondition(view.preferredSize(maxWidth: 1200).height > 0)
    print("repeatedBlankStatusReusesRows=true noNSPanelCreated=true")
  }
}
'''
presentation = declaration(source, "  private func presentConfiguredContent(")
presentation_test = r'''
import Foundation
import CoreGraphics
enum NSEvent { static let mouseLocation = NSPoint(x: 100, y: 100) }
struct NSScreen {
  let frame: NSRect; let visibleFrame: NSRect
  static let screens = [NSScreen(frame: NSRect(x: 0, y: 0, width: 1440, height: 900),
    visibleFrame: NSRect(x: 0, y: 0, width: 1440, height: 900))]
  static let main = screens.first
}
enum InputiaCandidatePanelDebug { static func log(_ text: @autoclosure () -> String) {} }
final class View {
  var frame = NSRect.zero, size = NSSize(width: 240, height: 23)
  var laidOut = false
  func preferredSize(maxWidth: CGFloat) -> NSSize { size }
  func debugSummary(maxWidth: CGFloat) -> String { "" }
  func layoutSubtreeIfNeeded() { laidOut = true }
  func displayIfNeeded() {}
}
final class Panel {
  let candidateView = View(), backgroundView = View()
  let minPanelWidth: CGFloat = 180, maxPanelWidth: CGFloat = 1600
  let expandedMaxPanelHeight: CGFloat = 220, collapsedMaxPanelHeight: CGFloat = 58
  let displaySettingsSnapshot: Int? = nil
  let displaySettings = (fontSize: CGFloat(14), unused: false)
  var settingsDidApply: ((Int) -> Void)?
  var frame = NSRect.zero, isVisible = false
  var frames = 0, fronts = 0
  func setFrame(_ value: NSRect, display: Bool) {
    precondition(!display, "不能在内容布局前同步显示新窗口frame")
    frame = value; frames += 1
  }
  func orderFrontRegardless() {
    precondition(candidateView.laidOut && candidateView.frame.size == frame.size)
    isVisible = true; fronts += 1
  }
  __PRESENT__
  func show(_ anchor: NSRect) { presentConfiguredContent(near: anchor, expanded: false, debugCandidates: []) }
}
@main struct PresentationCheck {
  static func main() {
    let p = Panel(), anchor = NSRect(x: 500, y: 600, width: 1, height: 18)
    p.show(anchor); precondition(p.frames == 1 && p.fronts == 1)
    p.show(anchor); precondition(p.frames == 1 && p.fronts == 1)
    p.show(anchor.offsetBy(dx: 40, dy: 0)); precondition(p.frames == 2 && p.fronts == 1)
    p.candidateView.size.width += 80
    p.show(anchor.offsetBy(dx: 40, dy: 0)); precondition(p.frames == 3 && p.fronts == 1)
    p.isVisible = false
    p.show(anchor.offsetBy(dx: 40, dy: 0)); precondition(p.frames == 3 && p.fronts == 2)
    print("sameFrameSkipsSetFrameAndOrderFront=true changedAnchorAndSizeUpdate=true hiddenWindowReopens=true layoutBeforeDisplay=true")
  }
}
'''.replace("__PRESENT__", presentation)
with tempfile.TemporaryDirectory(prefix="inputia-panel-refresh-") as directory:
    directory = Path(directory)
    content_path = directory / "Content.swift"
    content_path.write_text("import AppKit\nimport CoreText\n" + content + "\n" + content_test)
    binary = directory / "content"
    subprocess.run(["swiftc", "-parse-as-library", str(content_path),
        str(root / "Sources/InputiaInputMethod/InputiaExpandedCandidateGridNavigation.swift"),
        "-o", str(binary)], check=True)
    subprocess.run([str(binary)], check=True)
    presentation_path = directory / "Presentation.swift"
    presentation_path.write_text(presentation_test.replace("final class Panel {",
        declaration(source, "enum InputiaCandidatePanelPlacement") + "\nfinal class Panel {"))
    binary = directory / "presentation"
    subprocess.run(["swiftc", "-parse-as-library", str(presentation_path), "-o", str(binary)], check=True)
    subprocess.run([str(binary)], check=True)
