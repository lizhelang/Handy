import AppKit
import Foundation

enum InputiaVoiceInputLaunchResult: Equatable {
  case started(appPath: String, delayed: Bool)
  case missing
  case failed(message: String)
}

struct InputiaVoiceInputLaunchPlan: Equatable {
  let appPath: String
  let executablePath: String
  let delayed: Bool
  let startupArguments: [String]
  let toggleArguments: [String]
  let toggleDelaySeconds: TimeInterval
}

enum InputiaVoiceInputLauncher {
  static let handyBundleIdentifier = "com.pais.handy"
  static let handyExecutableName = "Handy"
  static let toggleTranscriptionArguments = ["--toggle-transcription"]
  static let startupArguments = ["--start-hidden"]
  static let startupToggleDelaySeconds: TimeInterval = 1.5

  static func candidateAppPaths(
    environment: [String: String] = ProcessInfo.processInfo.environment,
    homeDirectory: String = NSHomeDirectory(),
    workspaceAppPath: String? = NSWorkspace.shared.urlForApplication(
      withBundleIdentifier: handyBundleIdentifier
    )?.path
  ) -> [String] {
    uniquePaths([
      environment["INPUTIA_HANDY_APP"],
      workspaceAppPath,
      "/Applications/Handy.app",
      "\(homeDirectory)/Applications/Handy.app",
    ].compactMap { $0 })
  }

  static func executablePath(forAppPath appPath: String) -> String {
    "\(appPath)/Contents/MacOS/\(handyExecutableName)"
  }

  static func findHandyApp(
    environment: [String: String] = ProcessInfo.processInfo.environment,
    homeDirectory: String = NSHomeDirectory(),
    workspaceAppPath: String? = NSWorkspace.shared.urlForApplication(
      withBundleIdentifier: handyBundleIdentifier
    )?.path,
    fileExists: (String) -> Bool = { FileManager.default.fileExists(atPath: $0) }
  ) -> String? {
    candidateAppPaths(
      environment: environment,
      homeDirectory: homeDirectory,
      workspaceAppPath: workspaceAppPath
    ).first { appPath in
      fileExists(executablePath(forAppPath: appPath))
    }
  }

  static func launchPlan(appPath: String, isRunning: Bool) -> InputiaVoiceInputLaunchPlan {
    InputiaVoiceInputLaunchPlan(
      appPath: appPath,
      executablePath: executablePath(forAppPath: appPath),
      delayed: !isRunning,
      startupArguments: startupArguments,
      toggleArguments: toggleTranscriptionArguments,
      toggleDelaySeconds: isRunning ? 0 : startupToggleDelaySeconds
    )
  }

  static func isHandyRunning() -> Bool {
    !NSRunningApplication.runningApplications(withBundleIdentifier: handyBundleIdentifier).isEmpty
  }

  @discardableResult
  static func triggerVoiceInput() -> InputiaVoiceInputLaunchResult {
    guard let appPath = findHandyApp() else {
      return .missing
    }

    let plan = launchPlan(appPath: appPath, isRunning: isHandyRunning())
    if plan.delayed {
      let configuration = NSWorkspace.OpenConfiguration()
      configuration.arguments = plan.startupArguments
      configuration.activates = false
      configuration.hides = true
      NSWorkspace.shared.openApplication(
        at: URL(fileURLWithPath: plan.appPath),
        configuration: configuration
      ) { _, error in
        if let error {
          NSLog("Inputia failed to start Handy voice input app: \(error.localizedDescription)")
          return
        }
        DispatchQueue.global(qos: .userInitiated).asyncAfter(
          deadline: .now() + plan.toggleDelaySeconds
        ) {
          _ = runToggleProcess(executablePath: plan.executablePath)
        }
      }
      return .started(appPath: appPath, delayed: true)
    }

    if runToggleProcess(executablePath: plan.executablePath) {
      return .started(appPath: appPath, delayed: false)
    }
    return .failed(message: "无法运行 \(plan.executablePath) --toggle-transcription")
  }

  private static func runToggleProcess(executablePath: String) -> Bool {
    let task = Process()
    task.executableURL = URL(fileURLWithPath: executablePath)
    task.arguments = toggleTranscriptionArguments

    do {
      try task.run()
      task.waitUntilExit()
      return task.terminationStatus == 0
    } catch {
      NSLog("Inputia failed to toggle Handy voice input: \(error)")
      return false
    }
  }

  private static func uniquePaths(_ paths: [String]) -> [String] {
    var seen = Set<String>()
    var result: [String] = []
    for path in paths where !path.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
      if seen.insert(path).inserted {
        result.append(path)
      }
    }
    return result
  }
}
