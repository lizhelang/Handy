import AppKit
import Foundation
import OSLog
#if INPUTIA_PAIRED_BUILD
import Security
#endif

/// 自动准备只允许启动一次；明确退出或观察到进程结束后，不循环拉起服务。
struct InputiaVoiceServiceReadiness {
  private var attempted = false
  private var suspended = false
  mutating func requestStart(isRunning: Bool) -> Bool {
    guard !isRunning, !attempted, !suspended else { return false }
    attempted = true
    return true
  }
  mutating func suspend() { suspended = true }
}

/// 仅约束当前会话的首次 Fetch；一旦发起便保留 attempted，未知回执不能重试。
struct InputiaVoiceFirstFetchGate {
  private(set) var attempted = false
  mutating func claimAfterPolicyRefresh(verified: Bool) -> Bool {
    guard verified, !attempted else { return false }
    attempted = true
    return true
  }
}

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
  static let activeSessionMenuActions: Set<String> = ["status", "copy_latest", "history", "settings", "check_updates", "quit_service"]
  #if INPUTIA_PAIRED_BUILD
  private static let shortcutDiagnostic = Logger(subsystem: "com.inputia.shortcut", category: "control")
  private static let voiceQueue = DispatchQueue(label: "Inputia.unified-voice")
  private static let readinessQueue = DispatchQueue(label: "Inputia.service-readiness")
  private static var readiness = InputiaVoiceServiceReadiness()
  private static var serviceTerminationObserver: NSObjectProtocol?

  /// 输入法启用时异步准备同一签名配对服务，仅隐藏启动，不发送任何录音命令。
  static func ensureUnifiedServiceReady() {
    readinessQueue.async {
      do {
        let profile = InputiaProfile.current
        try profile.validateCandidatePaths()
        let bytes = try Data(contentsOf: profile.root.deletingLastPathComponent().appendingPathComponent("pair-manifest.json"))
        let manifest = try SignedPairManifest.verify(bytes, trust: InputiaEmbeddedPairTrust.trust)
        let identity = try manifest.identity(for: .handy)
        if serviceTerminationObserver == nil {
          serviceTerminationObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didTerminateApplicationNotification, object: nil, queue: nil
          ) { notification in
            guard let app = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication,
              app.bundleIdentifier == identity.identifier else { return }
            readinessQueue.async { readiness.suspend() }
          }
        }
        let running = !NSRunningApplication.runningApplications(withBundleIdentifier: identity.identifier).isEmpty
        guard readiness.requestStart(isRunning: running) else { return }
        let app = try verifiedInstalledService(identity: identity)
        openHiddenService(appPath: app.path) { error in
          if error != nil { NSLog("inputia_service_prepare_failed automatic_retry=false") }
          else { NSLog("inputia_service_hidden_start_requested recording_command_sent=false") }
        }
      } catch {
        NSLog("inputia_service_prepare_unavailable automatic_retry=false")
      }
    }
  }

  private static func verifiedInstalledService(identity: PairCodeIdentity) throws -> URL {
    let hashes = identity.cdhashes.map { "cdhash H\"\($0)\"" }.joined(separator: " or ")
    var requirement: SecRequirement?
    guard SecRequirementCreateWithString("identifier \"\(identity.identifier)\" and (\(hashes))" as CFString, [], &requirement) == errSecSuccess,
      let requirement else { throw InputiaVoiceServiceError.handshake }
    // 不采用 LaunchServices 可能指向构建备份的 URL，也不回落到日常 Handy 包。
    for path in installedServiceAppPaths() {
      let url = URL(fileURLWithPath: path).standardizedFileURL
      guard url.resolvingSymlinksInPath() == url,
        Bundle(url: url)?.bundleIdentifier == identity.identifier else { continue }
      var code: SecStaticCode?
      guard SecStaticCodeCreateWithPath(url as CFURL, [], &code) == errSecSuccess, let code,
        SecStaticCodeCheckValidity(code, SecCSFlags(rawValue: kSecCSCheckAllArchitectures), requirement) == errSecSuccess else { continue }
      var information: CFDictionary?
      guard SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &information) == errSecSuccess,
        let info = information as? [String: Any], let flags = info[kSecCodeInfoFlags as String] as? NSNumber,
        flags.uint32Value & 0x10000 != 0 else { continue }
      return url
    }
    throw InputiaVoiceServiceError.profile
  }
  private static var unifiedConnection: InputiaVoiceServiceConnection?
  private static var unifiedSession: String?
  private static var unifiedFetchGate = InputiaVoiceFirstFetchGate()
  private static var lastUnifiedPhase: String?
  private static var shortcutTerms: InputiaVoiceTermsVersion?
  private static var shortcutTarget: InputiaVoiceTarget?
  private static var shortcutServer: String?
  private static var shortcutSession: String?
  private static var shortcutDeliver: ((InputiaVoiceDelivery, @escaping (String) -> Void) -> Void)?
  private static var shortcutStatus: ((String) -> Void)?
  private static let shortcutQueue = DispatchQueue(label: "Inputia.shortcut-control")
  private static var shortcutConnection: InputiaVoiceServiceConnection?
  private static var shortcutLease: InputiaHostShortcutLease?
  private static var shortcutLeaseEpoch: UInt64 = 0
  private static var shortcutTimer: DispatchSourceTimer?
  private static var shortcutCycleBusy = false
  private static var shortcutRetryAfter: TimeInterval = 0
  private static var shortcutHasActiveOwner = false
  private static var shortcutProviderHadTarget: Bool?
  private static var sharedTermsRequestedAt: TimeInterval = 0
  private static var sharedTermsBusy = false
  private static let sharedTermsQueue = DispatchQueue(label: "Inputia.shared-terms")
  private static var sharedTermsConnection: InputiaVoiceServiceConnection?
  private struct Endpoint: Decodable { let profile_id: String; let protocol_major: Int; let server_instance: String; let socket_path: String }

  /// provider 和新会话复核在主线程；定时器、认证、数据库和 socket 均在后台。
  static func startShortcutListening(
    targetProvider: @escaping () -> InputiaVoiceTarget?,
    sharedTermsReceiver: @escaping (InputiaSharedTermsSnapshot, UInt64) -> Void,
    acceptStart: @escaping (InputiaHostShortcutTrigger, @escaping (Bool) -> Void) -> Void
  ) {
    shortcutQueue.async {
      guard shortcutTimer == nil else { return }
      let timer = DispatchSource.makeTimerSource(queue: shortcutQueue)
      timer.schedule(deadline: .now(), repeating: .milliseconds(200), leeway: .milliseconds(30))
      timer.setEventHandler {
        guard !shortcutCycleBusy, ProcessInfo.processInfo.systemUptime >= shortcutRetryAfter else { return }
        shortcutCycleBusy = true
        DispatchQueue.main.async {
          let target = targetProvider()
          if target == nil { InputiaSharedTermsMemory.shared.clear() }
          shortcutQueue.async {
            do {
              if shortcutProviderHadTarget != (target != nil) {
                shortcutProviderHadTarget = target != nil
                shortcutDiagnostic.notice("provider has_target=\(target != nil)")
              }
              if target == nil && !shortcutHasActiveOwner {
                shortcutConnection?.close()
                shortcutConnection = nil
                shortcutLease = nil
                shortcutCycleBusy = false
                return
              }
              if shortcutConnection == nil {
                // 没有可注册目标时不主动建立无用连接；已有连接仍可接收停止。
                guard target != nil || shortcutHasActiveOwner else { shortcutCycleBusy = false; return }
                shortcutConnection = try openAuthenticatedConnection()
              }
              guard let connection = shortcutConnection else { throw InputiaVoiceServiceError.handshake }
              if let target {
                let newTarget = shortcutLease?.target != target
                if shortcutLease?.target != target {
                  InputiaSharedTermsMemory.shared.clear()
                  guard shortcutLeaseEpoch < UInt64.max else { throw InputiaVoiceServiceError.policy }
                  shortcutLeaseEpoch += 1
                  let now = UInt64(max(0, Date().timeIntervalSince1970 * 1000))
                  shortcutLease = InputiaHostShortcutLease(lease_id: UUID().uuidString,
                    lease_epoch: shortcutLeaseEpoch, target: target,
                    issued_at_unix_ms: now, expires_at_unix_ms: now + 1000)
                }
                if let previous = shortcutLease {
                  let leaseStartedAt = ProcessInfo.processInfo.systemUptime
                  let now = UInt64(max(0, Date().timeIntervalSince1970 * 1000))
                  let renewed = InputiaHostShortcutLease(lease_id: previous.lease_id,
                    lease_epoch: previous.lease_epoch, target: previous.target,
                    issued_at_unix_ms: now, expires_at_unix_ms: now + 1000)
                  let leaseDeadline = leaseStartedAt + 1
                  try connection.registerShortcutLease(renewed)
                  if newTarget { shortcutDiagnostic.notice("target_registered field_observable=\(target.field_id != nil)") }
                  shortcutLease = renewed
                  let clock = ProcessInfo.processInfo.systemUptime
                  if !sharedTermsBusy, clock - sharedTermsRequestedAt >= 0.5,
                    connection.server.capabilities.contains("shared_terms_v1") {
                    sharedTermsRequestedAt = clock
                    sharedTermsBusy = true
                    let server = connection.server.instance_id
                    let version = connection.locallyAppliedVersion
                    let ticket = InputiaSharedTermsMemory.shared.ticket()
                    sharedTermsQueue.async {
                      defer { shortcutQueue.async { sharedTermsBusy = false } }
                      do {
                        if sharedTermsConnection == nil {
                          // 新连接先执行真实 barrier；下轮从屏障后的 generation 发起请求。
                          sharedTermsConnection = try openAuthenticatedConnection()
                          return
                        }
                        guard InputiaSharedTermsMemory.shared.ticket() == ticket else { return }
                        guard let termsConnection = sharedTermsConnection,
                          InputiaVoiceServiceConnection.sharedTermsConnectionMatches(
                            server: termsConnection.server.instance_id, primaryServer: server,
                            version: termsConnection.locallyAppliedVersion, primaryVersion: version)
                        else { throw InputiaVoiceServiceError.policy }
                        if let snapshot = try termsConnection.fetchSharedTerms(lease: renewed, leaseDeadline: leaseDeadline) {
                          DispatchQueue.main.async { sharedTermsReceiver(snapshot, ticket) }
                        }
                      } catch {
                        sharedTermsConnection?.close()
                        sharedTermsConnection = nil
                      }
                    }
                  }
                }
              }
              guard let trigger = try connection.pollShortcut(maxWaitMs: 50) else {
                shortcutCycleBusy = false; return
              }
              let finished: (Bool) -> Void = { accepted in
                shortcutQueue.async {
                  if trigger.starts_session && !accepted {
                    do {
                      if try connection.rejectUnconsumedShortcut(trigger.trigger_id) {
                        voiceQueue.async {
                          guard shortcutSession == trigger.session_id, shortcutServer == trigger.server_instance else { return }
                          clearShortcutOwnership(sessionID: trigger.session_id)
                        }
                      }
                    } catch {
                      // 撤销回执未知不宣称未执行；保持原操作身份，绝不换路开始。
                      NSLog("inputia_shortcut_rejection_unconfirmed automatic_replay=false")
                    }
                  }
                  shortcutCycleBusy = false
                }
              }
              if trigger.starts_session {
                guard let lease = shortcutLease, trigger.lease_id == lease.lease_id,
                  trigger.lease_epoch == lease.lease_epoch, trigger.target == lease.target,
                  target == lease.target else { finished(false); return }
                DispatchQueue.main.async { acceptStart(trigger, finished) }
              } else {
                // 已有会话的停止不依赖当前光标；真正使用的是首次开始时保存的回调。
                sendUnifiedShortcutTrigger(trigger, deliver: { _, ack in ack("pending_target") },
                  status: { _ in }, completion: finished)
              }
            } catch {
              shortcutConnection?.close()
              shortcutConnection = nil
              shortcutLease = nil
              shortcutCycleBusy = false
              shortcutRetryAfter = ProcessInfo.processInfo.systemUptime + 1
              // 重连仅恢复监听，不重放任何已经取出的触发。
              shortcutDiagnostic.notice("listener_unavailable automatic_trigger_replay=false")
            }
          }
        }
      }
      shortcutTimer = timer
      timer.resume()
      shortcutDiagnostic.notice("listener_started")
    }
  }

  private static func openAuthenticatedConnection() throws -> InputiaVoiceServiceConnection {
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
    do {
      guard connection.server.instance_id == endpoint.server_instance else { throw InputiaVoiceServiceError.handshake }
      try connection.synchronizePolicy(using: state)
      return connection
    } catch { connection.close(); throw error }
  }

  private static func clearShortcutOwnership(sessionID: String) {
    guard shortcutSession == sessionID else { return }
    if unifiedSession == sessionID {
      unifiedConnection?.close()
      unifiedConnection = nil
      unifiedSession = nil
    }
    shortcutSession = nil
    shortcutTarget = nil
    shortcutServer = nil
    shortcutTerms = nil
    shortcutDeliver = nil
    shortcutStatus = nil
    shortcutQueue.async { shortcutHasActiveOwner = false }
  }

  /// 新会话由调用方先在主线程复核预备目标；后续停止边沿不重新捕获当前输入框。
  static func sendUnifiedShortcutTrigger(_ trigger: InputiaHostShortcutTrigger,
    deliver: @escaping (InputiaVoiceDelivery, @escaping (String) -> Void) -> Void,
    status: @escaping (String) -> Void,
    completion: @escaping (Bool) -> Void) {
    voiceQueue.async {
      var touchedConnection = false
      var resumePolling = trigger.starts_session
      do {
        if trigger.starts_session {
          guard unifiedSession == nil else { throw InputiaVoiceServiceError.policy }
          let connection = try openAuthenticatedConnection()
          if let previous = shortcutSession {
            if shortcutServer == connection.server.instance_id {
              let old = try connection.request(sessionID: previous, requestID: UUID().uuidString, command: .status)
              guard old.view.map({ !["preparing", "recording", "processing"].contains($0.phase) }) == true else {
                connection.close(); throw InputiaVoiceServiceError.policy
              }
            }
            clearShortcutOwnership(sessionID: previous)
          }
          guard connection.server.instance_id == trigger.server_instance,
            let terms = connection.locallyAppliedVersion, terms.policy_epoch == trigger.policy_epoch else {
            connection.close(); throw InputiaVoiceServiceError.policy
          }
          // 写入前保留操作归属；回执未知后只查询事实，不重放新建会话。
          unifiedSession = trigger.session_id
          unifiedFetchGate = InputiaVoiceFirstFetchGate()
          unifiedConnection = connection
          touchedConnection = true
          shortcutSession = trigger.session_id
          shortcutQueue.async { shortcutHasActiveOwner = true }
          shortcutDeliver = deliver
          shortcutStatus = status
          shortcutTerms = terms
          shortcutTarget = trigger.target
          shortcutServer = trigger.server_instance
          lastUnifiedPhase = nil
        } else {
          guard shortcutSession == trigger.session_id,
            unifiedSession == nil || unifiedSession == trigger.session_id, shortcutTarget == trigger.target,
            shortcutServer == trigger.server_instance else { throw InputiaVoiceServiceError.policy }
          if unifiedConnection == nil {
            let recovered = try openAuthenticatedConnection()
            guard recovered.server.instance_id == trigger.server_instance else {
              recovered.close(); throw InputiaVoiceServiceError.handshake
            }
            let observed = try recovered.request(sessionID: trigger.session_id,
              requestID: UUID().uuidString, command: .status)
            guard observed.status == "session" else { recovered.close(); throw InputiaVoiceServiceError.handshake }
            guard let view = observed.view, ["preparing", "recording", "processing"].contains(view.phase) else {
              recovered.close()
              clearShortcutOwnership(sessionID: trigger.session_id)
              completion(true)
              return
            }
            unifiedConnection = recovered
            unifiedSession = trigger.session_id
            resumePolling = true
          }
          touchedConnection = true
        }
        guard let connection = unifiedConnection, let terms = shortcutTerms else {
          throw InputiaVoiceServiceError.policy
        }
        let reply = try connection.request(sessionID: trigger.session_id, requestID: trigger.trigger_id,
          command: .hostShortcut(target: trigger.target,
            postProcess: trigger.binding_id == "transcribe_with_post_process", terms: terms, edge: trigger.edge))
        guard reply.status == "session" else {
          if trigger.starts_session && reply.code != "unknown" {
            clearShortcutOwnership(sessionID: trigger.session_id)
          } else if let observed = try? connection.request(sessionID: trigger.session_id,
              requestID: UUID().uuidString, command: .status),
              let view = observed.view, !["preparing", "recording", "processing"].contains(view.phase) {
            clearShortcutOwnership(sessionID: trigger.session_id)
          }
          throw InputiaVoiceServiceError.handshake
        }
        if resumePolling, let originalDeliver = shortcutDeliver, let originalStatus = shortcutStatus {
          pollUnifiedVoice(connection: connection, session: trigger.session_id, target: trigger.target,
            deliver: originalDeliver, completion: originalStatus)
        }
        completion(true)
      } catch {
        if touchedConnection {
          unifiedConnection?.close()
          unifiedConnection = nil
        }
        DispatchQueue.main.async { status("快捷键会话结果未确认；未重放，也未改用其他插入方式。") }
        completion(false)
      }
    }
  }

  /// 统一菜单与语音共用串行通讯队列；不在系统菜单或按键回调等待服务。
  static func menuAction(kind: String, modelID: String? = nil, completion: @escaping (InputiaMenuReply?) -> Void) {
    if kind == "quit_service" { readinessQueue.async { readiness.suspend() } }
    voiceQueue.async {
      do {
        if let connection = unifiedConnection {
          guard activeSessionMenuActions.contains(kind) else {
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
        unifiedFetchGate = InputiaVoiceFirstFetchGate()
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
      guard unifiedSession == session, unifiedConnection === connection, !unifiedFetchGate.attempted else { return }
      var fetchConnection: InputiaVoiceServiceConnection?
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
          let refreshed: (InputiaVoiceServiceConnection, InputiaVoiceSessionView)
          do {
            refreshed = try refreshBeforeFirstFetch(connection: connection, session: session, target: target, previous: view)
          } catch {
            // 结束本地等待，但保留 shortcutSession/target 与持久 Prepared 结果；不重放。
            finishFailedPreFetch(connection)
            NSLog("inputia_voice_pre_fetch_refresh_failed fetch_attempted=false")
            DispatchQueue.main.async { completion("转写已保存，插入前策略同步未完成；未提取或重放结果。") }
            return
          }
          let outputConnection = refreshed.0
          guard unifiedFetchGate.claimAfterPolicyRefresh(verified: true) else { outputConnection.close(); return }
          fetchConnection = outputConnection
          unifiedConnection = outputConnection
          connection.close()
          // 只有唯一fetch成功才会取得正文；失联/重复请求绝不换路或再次fetch。
          guard let delivery = try outputConnection.fetchDelivery(view: refreshed.1, target: target) else {
            outputConnection.close(); unifiedConnection = nil; unifiedSession = nil
            clearShortcutOwnership(sessionID: session)
            DispatchQueue.main.async { completion("结果已有输出状态，未重复插入。请在 Inputia 查看历史。") }
            return
          }
          DispatchQueue.main.async {
            let acknowledge: (String) -> Void = { receipt in
              voiceQueue.async {
                guard unifiedSession == session, unifiedConnection === outputConnection else { return }
                do {
                  try outputConnection.acknowledgeDelivery(delivery, receipt: receipt)
                  NSLog("inputia_unified_voice_output_receipt=%@", receipt)
                  DispatchQueue.main.async {
                    completion(receipt == "dispatched" ? "已向原输入框派发文字。" : "结果保留在历史，未自动重试插入。")
                  }
                } catch {
                  NSLog("inputia_unified_voice_output_receipt_unknown automatic_replay=false")
                  DispatchQueue.main.async { completion("插入回执未知，未重放。请核对输入框和历史。") }
                }
                outputConnection.close(); unifiedConnection = nil; unifiedSession = nil
                clearShortcutOwnership(sessionID: session)
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
          clearShortcutOwnership(sessionID: session)
          DispatchQueue.main.async { completion(view.phase == "pending_target" ? "转写已保存到历史记录，结果待插入。" : "语音会话已结束。") }
        }
      } catch {
        (fetchConnection ?? connection).close(); unifiedConnection = nil; unifiedSession = nil
        NSLog("inputia_unified_voice_receipt_unknown automatic_replay=false")
        DispatchQueue.main.async { completion("语音回执未知，未重放请求。请在 Inputia 查看状态和历史。") }
      }
    }
  }

  private static func refreshBeforeFirstFetch(connection: InputiaVoiceServiceConnection, session: String,
    target: InputiaVoiceTarget, previous: InputiaVoiceSessionView) throws -> (InputiaVoiceServiceConnection, InputiaVoiceSessionView) {
    guard !unifiedFetchGate.attempted, unifiedSession == session, unifiedConnection === connection else {
      throw InputiaVoiceServiceError.policy
    }
    let refreshed = try openAuthenticatedConnection()
    do {
      guard refreshed.server.instance_id == connection.server.instance_id else { throw InputiaVoiceServiceError.handshake }
      let reply = try refreshed.request(sessionID: session, requestID: UUID().uuidString, command: .status)
      guard reply.status == "session", let current = reply.view, current.phase == "pending_target",
        current.session_id == session, previous.session_id == session,
        current.target_id == target.target_id, current.target_id == previous.target_id,
        current.item_id != nil, current.item_id == previous.item_id,
        current.output_operation_id != nil, current.output_operation_id == previous.output_operation_id,
        unifiedSession == session, unifiedConnection === connection, !unifiedFetchGate.attempted else {
        throw InputiaVoiceServiceError.policy
      }
      return (refreshed, current)
    } catch { refreshed.close(); throw error }
  }

  private static func finishFailedPreFetch(_ connection: InputiaVoiceServiceConnection?) {
    connection?.close()
    unifiedConnection = nil
    unifiedSession = nil
  }

  #if INPUTIA_CONNECTION_SELF_CHECK
  static func checkPreFetchFailureReleasesWait() -> Bool {
    let owner = "synthetic-pre-fetch-owner"
    unifiedSession = owner
    shortcutSession = owner
    finishFailedPreFetch(nil)
    let correct = unifiedSession == nil && unifiedConnection == nil && shortcutSession == owner
    shortcutSession = nil
    return correct
  }
  #endif
  #endif
  static let handyBundleIdentifier = "com.pais.handy"
  // 用户可见名称变更不擅自改协议/授权身份；发布程序仍兼容原bundle ID。
  static let inputiaBundleIdentifier = handyBundleIdentifier
  static let voiceServiceExecutableNames = ["handy", "Inputia", "Handy"]
  static let toggleTranscriptionArguments = ["--toggle-transcription"]
  static let startupArguments = ["--start-hidden"]
  static let startupToggleDelaySeconds: TimeInterval = 1.5

  static func installedServiceAppPaths(homeDirectory: String = NSHomeDirectory()) -> [String] {
    ["/Applications/Inputia Candidate.app", "\(homeDirectory)/Applications/Inputia Candidate.app"]
  }

  private static func openHiddenService(appPath: String, completion: @escaping (Error?) -> Void) {
    let configuration = NSWorkspace.OpenConfiguration()
    configuration.arguments = startupArguments
    configuration.activates = false
    configuration.hides = true
    configuration.createsNewApplicationInstance = false
    NSWorkspace.shared.openApplication(at: URL(fileURLWithPath: appPath), configuration: configuration) { _, error in
      completion(error)
    }
  }

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
      openHiddenService(appPath: plan.appPath) { error in
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
