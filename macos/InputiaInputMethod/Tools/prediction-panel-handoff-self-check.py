#!/usr/bin/env python3
"""提取生产过渡绘制调用到无窗口替身，检查原窗连续性与非候选状态。"""

from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
source = (root / "Sources/InputiaInputMethod/InputiaCandidatePanel.swift").read_text()


def declaration(signature: str) -> str:
    start = source.index(signature)
    brace = source.index("{", start)
    depth = 1
    end = brace + 1
    while depth:
        if source[end] == "{":
            depth += 1
        elif source[end] == "}":
            depth -= 1
        end += 1
    return source[start:end]


item = declaration("private struct InputiaCandidateDisplayItem")
clamped = declaration("private extension Comparable")
configure = declaration("  func configureStatus(")
methods = "\n".join(declaration(signature) for signature in [
    "  func show(\n",
    "  func showPredictionPending(",
    "  func showPredictionEmpty(",
    "  private func showPredictionStatus(",
    "  func clearForRestoration()",
    "  func isRestorationCurrent(",
])
fixture = r'''
private final class Content {
  var expanded = true
  var fontSize: CGFloat = 14
  var primaryCandidateCount = 1
  var activeRowIndex = 1
  var rows = [[InputiaCandidateDisplayItem(number: 1, text: "旧候选", isHighlighted: true)]]
  var needsLayout = false
  var rebuilds = 0
  var layouts = 0
  func rebuildItemViews() { rebuilds += 1 }
  func layoutSubtreeIfNeeded() { layouts += 1 }
  func displayIfNeeded() {}
  func configure(candidates: [String], expanded: Bool, fontSize: CGFloat,
    primaryCandidateCount: Int?, activeRowIndex: Int) {
    rows = [candidates.map { InputiaCandidateDisplayItem(number: 1, text: $0, isHighlighted: true) }]
  }
  __CONFIGURE__
}
private final class Panel {
  static let collapsedCandidateLimit = 9
  static let expandedCandidateLimit = 40
  var isVisible = true
  let candidateView = Content()
  let displaySettings = (fontSize: CGFloat(14), unused: false)
  var frame = NSRect(x: 400, y: 300, width: 620, height: 23)
  var restorationSequence: UInt64 = 0
  var restorationToken: UInt64?
  var presentCount = 0
  var hideCount = 0
  var lastAnchor: NSRect?
  func reloadDisplaySettingsIfNeeded() {}
  func presentConfiguredContent(near anchor: NSRect, expanded: Bool, debugCandidates: [String]) {
    presentCount += 1; lastAnchor = anchor; isVisible = true
  }
  func hide() { restorationToken = nil; hideCount += 1; isVisible = false }
  __METHODS__
}
@main
struct HandoffSelfCheck {
  static func main() {
    var failures: [String] = []
    func check(_ name: String, _ value: Bool) {
      print("\(name)=\(value)")
      if !value { failures.append(name) }
    }
    let panel = Panel()
    let identity = ObjectIdentifier(panel)
    let frame = panel.frame
    let anchor = NSRect(x: 410, y: 350, width: 1, height: 18)
    panel.showPredictionPending(near: anchor)
    check("predictionPendingKeepsVisibleWindow", panel.isVisible && panel.hideCount == 0
      && panel.presentCount == 0 && panel.frame == frame && ObjectIdentifier(panel) == identity)
    check("predictionPendingRemovesOldCandidateIdentity", panel.candidateView.rows.count == 1
      && panel.candidateView.rows[0].count == 1 && panel.candidateView.rows[0][0].text.isEmpty
      && panel.candidateView.rows[0][0].number == nil && !panel.candidateView.rows[0][0].isHighlighted
      && panel.candidateView.primaryCandidateCount == 0 && !panel.candidateView.expanded)
    panel.showPredictionEmpty(near: anchor)
    check("predictionEmptyKeepsSameVisibleWindow", panel.isVisible && panel.hideCount == 0
      && panel.presentCount == 0 && panel.frame == frame && ObjectIdentifier(panel) == identity)
    check("predictionEmptyRemainsUnselectable", panel.candidateView.rows[0][0].text == "暂无后续联想"
      && panel.candidateView.rows[0][0].number == nil && !panel.candidateView.rows[0][0].isHighlighted)
    check("predictionStatusesUpdateLayout", panel.candidateView.rebuilds == 2
      && panel.candidateView.layouts == 2 && panel.candidateView.needsLayout)
    panel.show(candidates: ["后续推荐"], near: anchor)
    check("predictionReplyUpdatesWithoutHideOrRecreation", panel.hideCount == 0 && panel.presentCount == 1
      && panel.isVisible && ObjectIdentifier(panel) == identity
      && panel.candidateView.rows[0][0].text == "后续推荐")
    let hidden = Panel(); hidden.isVisible = false
    hidden.showPredictionPending(near: anchor)
    check("predictionPendingCanOpenMissingPanel", hidden.isVisible && hidden.presentCount == 1
      && hidden.hideCount == 0 && hidden.lastAnchor == anchor)
    let restoring = Panel()
    let oldFrame = restoring.frame
    let token = restoring.clearForRestoration()
    check("restorationClearsPrivateRowsWithoutClosing", restoring.isRestorationCurrent(token)
      && restoring.candidateView.rows[0][0].text.isEmpty
      && restoring.candidateView.rows[0][0].number == nil
      && restoring.frame == oldFrame && restoring.hideCount == 0 && restoring.presentCount == 0)
    let newer = restoring.clearForRestoration()
    check("newRestorationInvalidatesPrevious", !restoring.isRestorationCurrent(token)
      && restoring.isRestorationCurrent(newer))
    restoring.show(candidates: ["新候选"], near: anchor)
    check("newCandidateInvalidatesRestoration", !restoring.isRestorationCurrent(newer))
    let statusToken = restoring.clearForRestoration()
    restoring.showPredictionEmpty(near: anchor)
    check("newStatusInvalidatesRestoration", !restoring.isRestorationCurrent(statusToken))
    let hideToken = restoring.clearForRestoration()
    restoring.hide()
    check("hideInvalidatesRestoration", !restoring.isRestorationCurrent(hideToken))
    print("predictionPanelHandoffSelfCheckPassed=\(failures.isEmpty)")
    if !failures.isEmpty { exit(1) }
  }
}
'''.replace("__CONFIGURE__", configure).replace("__METHODS__", methods)
with tempfile.TemporaryDirectory(prefix="inputia-prediction-handoff-") as temporary:
    directory = Path(temporary)
    swift = directory / "HandoffSelfCheck.swift"
    swift.write_text("import Foundation\nimport CoreGraphics\n" + item + "\n" + clamped + fixture)
    binary = directory / "handoff-self-check"
    subprocess.run(["/usr/bin/swiftc", "-parse-as-library", str(swift), "-o", str(binary)], check=True)
    raise SystemExit(subprocess.run([str(binary)], check=False).returncode)
