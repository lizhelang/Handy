import AppKit
import Foundation

private struct InputiaAppCandidate {
  let path: String
  let source: String
}

private let launcherVersion =
  Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? ""
private let inputiaAppCandidates: [InputiaAppCandidate] = [
  ProcessInfo.processInfo.environment["INPUTIA_APP"].map {
    InputiaAppCandidate(path: $0, source: "INPUTIA_APP")
  },
  InputiaAppCandidate(path: "/Library/Input Methods/InputiaInputMethod.app", source: "system"),
  InputiaAppCandidate(
    path: "\(NSHomeDirectory())/Library/Input Methods/InputiaInputMethod.app",
    source: "user"
  ),
].compactMap { $0 }

private func showFailure(message: String) {
  let app = NSApplication.shared
  app.setActivationPolicy(.regular)
  app.activate(ignoringOtherApps: true)

  let alert = NSAlert()
  alert.messageText = "无法打开 Inputia 设置"
  alert.informativeText = message
  alert.alertStyle = .warning
  alert.addButton(withTitle: "好")
  alert.runModal()
}

private func openSettingsApp(at appPath: String) -> Bool {
  let task = Process()
  task.executableURL = URL(fileURLWithPath: "/usr/bin/open")
  task.arguments = ["-n", appPath, "--args", "--open-settings"]

  do {
    try task.run()
    task.waitUntilExit()
    return task.terminationStatus == 0
  } catch {
    NSLog("Inputia settings launcher failed: \(error)")
    return false
  }
}

private func bundleVersion(at appPath: String) -> String? {
  Bundle(url: URL(fileURLWithPath: appPath))?.object(forInfoDictionaryKey: "CFBundleVersion")
    as? String
}

private func matchingCandidate() -> InputiaAppCandidate? {
  for candidate in inputiaAppCandidates
  where FileManager.default.fileExists(atPath: candidate.path) {
    guard !launcherVersion.isEmpty else {
      return candidate
    }

    if bundleVersion(at: candidate.path) == launcherVersion {
      return candidate
    }
  }

  return nil
}

private func candidateReport() -> String {
  inputiaAppCandidates
    .map { candidate in
      let version = bundleVersion(at: candidate.path) ?? "missing"
      return "\(candidate.source): v\(version) \(candidate.path)"
    }
    .joined(separator: "\n")
}

@main
struct InputiaSettingsLauncher {
  static func main() {
    if let candidate = matchingCandidate(), openSettingsApp(at: candidate.path) {
      return
    }

    if launcherVersion.isEmpty {
      showFailure(message: "没有找到可打开的 InputiaInputMethod.app。请先安装新版 Inputia 输入法包。")
    } else {
      showFailure(
        message:
          "Inputia 设置启动器是 v\(launcherVersion)，但没有找到同版本的 InputiaInputMethod.app。请重新安装同一个新版安装包。\n\n当前检测到：\n\(candidateReport())"
      )
    }
  }
}
