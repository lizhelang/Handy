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
  #if INPUTIA_PAIRED_BUILD
  private static let voiceQueue = DispatchQueue(label: "Inputia.unified-voice")
  private static var unifiedConnection: InputiaVoiceServiceConnection?
  private static var unifiedSession: String?
  private static var lastUnifiedPhase: String?
  private struct Endpoint: Decodable { let profile_id: String; let protocol_major: Int; let server_instance: String; let socket_path: String }

  /// 实际菜单仅入队，绝不在InputMethodKit主线程等待IPC或数据库。
  static func triggerUnifiedVoice(target: InputiaVoiceTarget?, completion: @escaping (String) -> Void) {
    voiceQueue.async {
      var stage = "candidate_profile"
      do {
        let profile = InputiaProfile.current
        try profile.validateCandidatePaths()
        if let session = unifiedSession, let connection = unifiedConnection {
          stage = "stop_request"
          let reply = try connection.request(sessionID: session, requestID: UUID().uuidString, command: .stop)
          guard reply.status == "session" else { throw InputiaVoiceServiceError.handshake }
          return
        }
        guard let target else { throw InputiaVoiceServiceError.policy }
        stage = "read_endpoint"
        let endpoint = try JSONDecoder().decode(Endpoint.self,
          from: Data(contentsOf: profile.handyRoot.appendingPathComponent("integration-endpoint.json"), options: .mappedIfSafe))
        guard endpoint.profile_id == "unified-candidate:\(profile.runID ?? "")", endpoint.protocol_major == 1 else { throw InputiaVoiceServiceError.profile }
        let manifestURL = profile.root.deletingLastPathComponent().appendingPathComponent("pair-manifest.json")
        stage = "read_manifest"
        let manifest = try Data(contentsOf: manifestURL)
        guard manifest.count <= 16_384 else { throw InputiaVoiceServiceError.handshake }
        stage = "open_shared_state"
        let state = try InputiaVoiceSharedState(profile: profile)
        stage = "authenticate_and_handshake"
        let connection = try InputiaVoiceServiceConnection.connect(endpoint: endpoint.socket_path, signedManifest: manifest,
          trust: InputiaEmbeddedPairTrust.trust, profile: profile, previousEpoch: state.lastVersion().policy_epoch)
        guard connection.server.instance_id == endpoint.server_instance else { connection.close(); throw InputiaVoiceServiceError.handshake }
        stage = "policy_barrier"
        try connection.synchronizePolicy(using: state)
        guard let version = connection.locallyAppliedVersion else { throw InputiaVoiceServiceError.policy }
        let session = UUID().uuidString
        stage = "start_request"
        let reply = try connection.request(sessionID: session, requestID: UUID().uuidString,
          command: .start(target: target, postProcess: false, terms: version))
        guard reply.status == "session" else { connection.close(); throw InputiaVoiceServiceError.handshake }
        unifiedConnection = connection
        unifiedSession = session
        lastUnifiedPhase = nil
        DispatchQueue.main.async { completion("正在准备，再次点击停止") }
        pollUnifiedVoice(connection: connection, session: session, completion: completion)
      } catch {
        unifiedConnection?.close(); unifiedConnection = nil; unifiedSession = nil
        NSLog("inputia_unified_voice_entry_failed stage=%@", stage)
        DispatchQueue.main.async { completion("连接、权限或策略同步未完成；没有改用日常 Handy 或其他插入路线。") }
      }
    }
  }

  private static func pollUnifiedVoice(connection: InputiaVoiceServiceConnection, session: String, completion: @escaping (String) -> Void) {
    voiceQueue.asyncAfter(deadline: .now() + 0.25) {
      guard unifiedSession == session else { return }
      do {
        let reply = try connection.request(sessionID: session, requestID: UUID().uuidString, command: .status)
        guard let view = reply.view, reply.status == "session" else { throw InputiaVoiceServiceError.handshake }
        if lastUnifiedPhase != view.phase {
          lastUnifiedPhase = view.phase
          NSLog("inputia_unified_voice_phase=%@", view.phase)
          if view.phase == "recording" { DispatchQueue.main.async { completion("录音中，再次点击停止") } }
        }
        if ["preparing", "recording", "processing"].contains(view.phase) {
          pollUnifiedVoice(connection: connection, session: session, completion: completion)
        } else {
          NSLog("inputia_unified_voice_session_terminal phase=%@", view.phase)
          connection.close(); unifiedConnection = nil; unifiedSession = nil
          DispatchQueue.main.async { completion(view.phase == "pending_target" ? "转写已保存到统一历史，结果待插入。" : "语音会话已结束。") }
        }
      } catch {
        connection.close(); unifiedConnection = nil; unifiedSession = nil
        NSLog("inputia_unified_voice_receipt_unknown automatic_replay=false")
        DispatchQueue.main.async { completion("语音回执未知，未重放请求。请在 Handy 查看状态和历史。") }
      }
    }
  }
  #endif
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
