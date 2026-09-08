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

  /// 统一菜单与语音共用串行通讯队列；不在系统菜单或按键回调等待服务。
  static func menuAction(kind: String, modelID: String? = nil, completion: @escaping (InputiaMenuReply?) -> Void) {
    voiceQueue.async {
      do {
        if let connection = unifiedConnection {
          guard ["status", "copy_latest", "history", "settings", "check_updates"].contains(kind) else {
            DispatchQueue.main.async { completion(nil) }; return
          }
          let reply = try connection.menuRequest(kind: kind, modelID: modelID)
          DispatchQueue.main.async { completion(reply.status == "menu" ? reply : nil) }
          return
        }
        let profile = InputiaProfile.current
        try profile.validateCandidatePaths()
        let endpoint = try JSONDecoder().decode(Endpoint.self,
          from: Data(contentsOf: profile.handyRoot.appendingPathComponent("integration-endpoint.json")))
        guard endpoint.protocol_major == 1,
          endpoint.profile_id == "unified-candidate:\(profile.runID ?? "")" else { throw InputiaVoiceServiceError.profile }
        let manifest = try Data(contentsOf: profile.root.deletingLastPathComponent().appendingPathComponent("pair-manifest.json"))
        guard manifest.count <= 16_384 else { throw InputiaVoiceServiceError.handshake }
        let state = try InputiaVoiceSharedState(profile: profile)
        let connection = try InputiaVoiceServiceConnection.connect(endpoint: endpoint.socket_path,
          signedManifest: manifest, trust: InputiaEmbeddedPairTrust.trust, profile: profile,
          previousEpoch: state.lastVersion().policy_epoch)
        defer { connection.close() }
        guard connection.server.instance_id == endpoint.server_instance else { throw InputiaVoiceServiceError.handshake }
        try connection.synchronizePolicy(using: state)
        let reply = try connection.menuRequest(kind: kind, modelID: modelID)
        DispatchQueue.main.async { completion(reply.status == "menu" ? reply : nil) }
      } catch {
        NSLog("inputia_menu_request_unconfirmed automatic_replay=false")
        DispatchQueue.main.async { completion(nil) }
      }
    }
  }

  /// 实际菜单仅入队，绝不在InputMethodKit主线程等待IPC或数据库。
  static func triggerUnifiedVoice(target: InputiaVoiceTarget?,
    deliver: @escaping (InputiaVoiceDelivery, @escaping (String) -> Void) -> Void,
    completion: @escaping (String) -> Void) {
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
        pollUnifiedVoice(connection: connection, session: session, target: target, deliver: deliver, completion: completion)
      } catch {
        unifiedConnection?.close(); unifiedConnection = nil; unifiedSession = nil
        NSLog("inputia_unified_voice_entry_failed stage=%@", stage)
        DispatchQueue.main.async { completion("连接、权限或策略同步未完成；没有改用其他插入路线。") }
      }
    }
  }

  private static func pollUnifiedVoice(connection: InputiaVoiceServiceConnection, session: String,
    target: InputiaVoiceTarget,
    deliver: @escaping (InputiaVoiceDelivery, @escaping (String) -> Void) -> Void,
    completion: @escaping (String) -> Void) {
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
          pollUnifiedVoice(connection: connection, session: session, target: target, deliver: deliver, completion: completion)
        } else if view.phase == "pending_target", target.field_id != nil {
          // 只有唯一fetch成功才会取得正文；失联/重复请求绝不换路或再次fetch。
          guard let delivery = try connection.fetchDelivery(view: view, target: target) else {
            connection.close(); unifiedConnection = nil; unifiedSession = nil
            DispatchQueue.main.async { completion("结果已有输出状态，未重复插入。请在 Inputia 查看历史。") }
            return
          }
          DispatchQueue.main.async {
            let acknowledge: (String) -> Void = { receipt in
              voiceQueue.async {
                guard unifiedSession == session else { return }
                do {
                  try connection.acknowledgeDelivery(delivery, receipt: receipt)
                  NSLog("inputia_unified_voice_output_receipt=%@", receipt)
                  DispatchQueue.main.async {
                    completion(receipt == "dispatched" ? "已向原输入框派发文字。" : "结果保留在历史，未自动重试插入。")
                  }
                } catch {
                  NSLog("inputia_unified_voice_output_receipt_unknown automatic_replay=false")
                  DispatchQueue.main.async { completion("插入回执未知，未重放。请核对输入框和历史。") }
                }
                connection.close(); unifiedConnection = nil; unifiedSession = nil
              }
            }
            guard ProcessInfo.processInfo.systemUptime < delivery.dispatchDeadline else {
              acknowledge("pending_target"); return
            }
            deliver(delivery, acknowledge)
          }
        } else {
          NSLog("inputia_unified_voice_session_terminal phase=%@", view.phase)
          connection.close(); unifiedConnection = nil; unifiedSession = nil
          DispatchQueue.main.async { completion(view.phase == "pending_target" ? "转写已保存到历史记录，结果待插入。" : "语音会话已结束。") }
        }
      } catch {
        connection.close(); unifiedConnection = nil; unifiedSession = nil
        NSLog("inputia_unified_voice_receipt_unknown automatic_replay=false")
        DispatchQueue.main.async { completion("语音回执未知，未重放请求。请在 Inputia 查看状态和历史。") }
      }
    }
  }
  #endif
  static let handyBundleIdentifier = "com.pais.handy"
  // 用户可见名称变更不擅自改协议/授权身份；发布程序仍兼容原bundle ID。
  static let inputiaBundleIdentifier = handyBundleIdentifier
  static let voiceServiceExecutableNames = ["handy", "Inputia", "Handy"]
  static let toggleTranscriptionArguments = ["--toggle-transcription"]
  static let startupArguments = ["--start-hidden"]
  static let startupToggleDelaySeconds: TimeInterval = 1.5

  static func candidateAppPaths(
    environment: [String: String] = ProcessInfo.processInfo.environment,
    homeDirectory: String = NSHomeDirectory(),
    workspaceAppPath: String? = NSWorkspace.shared.urlForApplication(
      withBundleIdentifier: inputiaBundleIdentifier
    )?.path,
    legacyWorkspaceAppPath: String? = NSWorkspace.shared.urlForApplication(
      withBundleIdentifier: handyBundleIdentifier
    )?.path
  ) -> [String] {
    uniquePaths([
      environment["INPUTIA_HANDY_APP"],
      workspaceAppPath,
      legacyWorkspaceAppPath,
      "/Applications/Inputia.app",
      "\(homeDirectory)/Applications/Inputia.app",
      "/Applications/Handy.app",
      "\(homeDirectory)/Applications/Handy.app",
    ].compactMap { $0 })
  }

  static func executablePaths(forAppPath appPath: String) -> [String] {
    voiceServiceExecutableNames.map { executableName in
      "\(appPath)/Contents/MacOS/\(executableName)"
    }
  }

  static func executablePath(
    forAppPath appPath: String,
    fileExists: (String) -> Bool = { FileManager.default.fileExists(atPath: $0) }
  ) -> String {
    executablePaths(forAppPath: appPath).first(where: fileExists)
      ?? "\(appPath)/Contents/MacOS/Inputia"
  }

  static func findHandyApp(
    environment: [String: String] = ProcessInfo.processInfo.environment,
    homeDirectory: String = NSHomeDirectory(),
    workspaceAppPath: String? = NSWorkspace.shared.urlForApplication(
      withBundleIdentifier: inputiaBundleIdentifier
    )?.path,
    legacyWorkspaceAppPath: String? = NSWorkspace.shared.urlForApplication(
      withBundleIdentifier: handyBundleIdentifier
    )?.path,
    fileExists: (String) -> Bool = { FileManager.default.fileExists(atPath: $0) }
  ) -> String? {
    candidateAppPaths(
      environment: environment,
      homeDirectory: homeDirectory,
      workspaceAppPath: workspaceAppPath,
      legacyWorkspaceAppPath: legacyWorkspaceAppPath
    ).first { appPath in
      executablePaths(forAppPath: appPath).contains(where: fileExists)
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
          NSLog("Inputia failed to start voice service: \(error.localizedDescription)")
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
      NSLog("Inputia failed to toggle voice input: \(error)")
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
