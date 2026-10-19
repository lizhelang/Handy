#!/usr/bin/env python3
"""只编译生产纯定位函数及共用回归用例，不创建 AppKit 窗口或读取本机输入状态。"""

from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
source = (root / "Sources/InputiaInputMethod/InputiaCandidatePanel.swift").read_text()
checks = (root / "Tools/InputiaCandidatePanelLayoutSelfCheck.swift").read_text()
placement = source[source.index("enum InputiaCandidatePanelPlacement {") : source.index("final class InputiaCandidatePanel: NSPanel {")]
checks = checks[checks.index("enum InputiaCandidatePanelPlacementChecks {") : checks.index("@main")]
runner = r'''
@main
struct PlacementSelfCheck {
  static func main() {
    var failures: [String] = []
    InputiaCandidatePanelPlacementChecks.run { name, passed in
      print("\(name)=\(passed)")
      if !passed { failures.append(name) }
    }
    print("candidatePanelPlacementSelfCheckPassed=\(failures.isEmpty)")
    if !failures.isEmpty { exit(1) }
  }
}
'''
with tempfile.TemporaryDirectory(prefix="inputia-candidate-placement-") as temporary:
    directory = Path(temporary)
    swift = directory / "PlacementSelfCheck.swift"
    swift.write_text("import Foundation\nimport CoreGraphics\n" + placement + checks + runner)
    binary = directory / "placement-self-check"
    subprocess.run(["/usr/bin/swiftc", "-parse-as-library", str(swift), "-o", str(binary)], check=True)
    raise SystemExit(subprocess.run([str(binary)], check=False).returncode)
