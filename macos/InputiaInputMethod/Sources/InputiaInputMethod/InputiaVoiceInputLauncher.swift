import AppKit
import Foundation
import OSLog
#if INPUTIA_PAIRED_BUILD
import Security
#endif

/// 白名单诊断仅记录控制阶段、固定拒绝码与计数，永不写正文/拼音/窗口或目标身份。
enum InputiaPersonalizationDiagnostics {
  private static let log = Logger(subsystem: "com.inputia.personalization", category: "pipeline")
  private static let lock = NSLock()
  private static var previous: [String: String] = [:]
  private static let phases: Set<String> = ["policy", "query", "schedule", "capture_guard", "capture_reply",
    "target_capture", "target_validate", "target_transport", "personal_transport", "commit", "typed_result"]
  private static let reasons: Set<String> = ["ok", "no_reply", "expired", "disabled", "not_active", "mode", "page", "expanded", "recall", "prediction_pending",
    "unavailable", "missing_client", "missing_bundle", "invalid_selection", "missing_target", "target_identity", "activation", "composition", "selection_generation", "client_identity", "selection_changed", "remote_selection_mismatch", "history_only", "policy", "stale", "accepted",
    "missing_selection", "missing_origin", "origin_mismatch", "text_mismatch", "invalid_start", "selection_not_committed", "profile", "handshake", "io", "timeout", "invalid_frame", "main_thread", "decode", "other_error", "unknown_code",
    "accessibility_permission_required", "field_unobservable", "secure_input_enabled", "focused_application_mismatch", "secure_text_field", "field_focus_changed",
    "target_dispatch_replayed", "target_entropy_unavailable", "target_expired", "target_history_only", "target_main_unavailable", "target_operation_missing", "target_owner_mismatch", "target_process_changed", "target_query_busy", "target_query_timeout", "target_registry_full", "target_retirement_pending", "target_retirement_timeout_restart_required", "target_retirement_unavailable", "target_sensitive_source", "target_source_changed", "target_source_missing", "target_unknown",
    "personalization_disabled_or_changed", "personalization_policy_changed", "personalization_secure_input", "personalization_source_missing", "prediction_no_longer_available"]
  static func record(_ phase: String, _ reason: String, flags: Int = 0, count: Int = 0) {
    let phase = phases.contains(phase) ? phase : "unavailable"
    let reason = reasons.contains(reason) ? reason : "unknown_code"
    let value = "\(reason):\(flags):\(count)"
    lock.lock()
    let changed = previous[phase] != value
    previous[phase] = value
    lock.unlock()
    guard changed else { return }
    log.notice("phase=\(phase, privacy: .public) reason=\(reason, privacy: .public) flags=\(flags, privacy: .public) count=\(count, privacy: .public)")
  }
  #if INPUTIA_PAIRED_BUILD
  static func errorReason(_ error: Error) -> String {
    switch error {
    case InputiaVoiceServiceError.profile: return "profile"
    case InputiaVoiceServiceError.handshake: return "handshake"
    case InputiaVoiceServiceError.policy: return "policy"
    case InputiaConnectionError.mainThread: return "main_thread"
    case InputiaConnectionError.endpoint: return "profile"
    case InputiaConnectionError.io: return "io"
    case InputiaConnectionError.timeout: return "timeout"
    case InputiaConnectionError.invalidFrame: return "invalid_frame"
    case is DecodingError: return "decode"
    default: return "other_error"
    }
  }
  #endif
}

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
  mutating func resumeForExplicitStart() { attempted = false; suspended = false }
  var needsAutomaticPreparation: Bool { !attempted && !suspended }
  mutating func markServiceObserved() { attempted = true }
}

enum InputiaVoiceServiceMenuState: Equatable {
  case running, stopped, unavailable
  var action: String? {
    switch self { case .running: return "quit_service"; case .stopped: return "start_service"; case .unavailable: return nil }
  }
  var title: String {
    switch self {
    case .running: return "退出语音服务（保留基础输入）"
    case .stopped: return "打开语音服务"
    case .unavailable: return "语音服务状态不可用"
    }
  }
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

/// 回执归属独立于可退休的全局会话；调用仍限于串行语音队列。
final class InputiaVoiceReceiptGate {
  private var claimed = false
  func claim() -> Bool {
    guard !claimed else { return false }
    claimed = true
    return true
  }
}

/// 只复用已经完整认证的 socket；权限或配对清单变化使该租约失效。
struct InputiaAuthenticatedConnectionScope {
  let permissionEpoch: UInt64
  let manifest: Data
  let server: String
  func matches(epoch: UInt64, manifest: Data, expectedServer: String?) -> Bool {
    permissionEpoch == epoch && self.manifest == manifest && (expectedServer == nil || expectedServer == server)
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
  private static let readinessLaunches = DispatchGroup()
  private static var explicitStartPending = false
  private static var serviceTerminationObserver: NSObjectProtocol?

  private struct BusinessConnection {
    let connection: InputiaVoiceServiceConnection
    let scope: InputiaAuthenticatedConnectionScope
  }
  private static var personalizationConnection: BusinessConnection?
  private static var typedCaptureConnection: BusinessConnection?
  private static func reusableBusinessConnection(_ cached: inout BusinessConnection?, epoch: UInt64,
    expectedServer: String?) throws -> InputiaVoiceServiceConnection {
    guard InputiaPermissionLifecycle.shared.epoch == epoch,
      InputiaPermissionLifecycle.shared.allowsServiceConnection,
      InputiaPermissionLifecycle.shared.backgroundMaintenanceAllowsWork() else { throw InputiaVoiceServiceError.policy }
    let manifest = try InputiaProfile.current.readPairManifest()
    guard manifest.count <= 16_384 else { throw InputiaVoiceServiceError.handshake }
    if let previous = cached, previous.scope.matches(epoch: epoch, manifest: manifest, expectedServer: expectedServer) {
      return previous.connection
    }
    cached?.connection.closeTypedCaptureConnection(); cached = nil
    let connection = try openAuthenticatedConnection()
    guard InputiaPermissionLifecycle.shared.epoch == epoch,
      expectedServer == nil || expectedServer == connection.server.instance_id else {
      connection.closeTypedCaptureConnection(); throw InputiaVoiceServiceError.policy
    }
    cached = BusinessConnection(connection: connection,
      scope: InputiaAuthenticatedConnectionScope(permissionEpoch: epoch, manifest: manifest, server: connection.server.instance_id))
    return connection
  }
  private static func retireBusinessConnections() {
    personalizationQueue.async {
      personalizationConnection?.connection.closeTypedCaptureConnection(); personalizationConnection = nil
    }
    typedCaptureQueue.async {
      typedCaptureConnection?.connection.closeTypedCaptureConnection(); typedCaptureConnection = nil
    }
  }
  private static let personalizationQueue = DispatchQueue(label: "Inputia.personalization")
  static func personalization(_ command: InputiaPersonalCommand, deadline: TimeInterval,
    expectedServer: String? = nil, completion: @escaping (InputiaPersonalReply?) -> Void) {
    let permissionEpoch = InputiaPermissionLifecycle.shared.epoch
    personalizationQueue.async {
      var reply: InputiaPersonalReply?
      if ProcessInfo.processInfo.systemUptime < deadline {
        do {
          let connection = try reusableBusinessConnection(&personalizationConnection, epoch: permissionEpoch, expectedServer: expectedServer)
          if ProcessInfo.processInfo.systemUptime < deadline,
            expectedServer == nil || expectedServer == connection.server.instance_id {
            reply = try connection.personalization(command)
          }
          InputiaPersonalizationDiagnostics.record("personal_transport", reply == nil ? "stale" : "ok")
        } catch {
          personalizationConnection?.connection.closeTypedCaptureConnection(); personalizationConnection = nil
          InputiaPersonalizationDiagnostics.record("personal_transport", InputiaPersonalizationDiagnostics.errorReason(error))
        }
      } else { InputiaPersonalizationDiagnostics.record("personal_transport", "expired") }
      let result = reply
      InputiaPersonalMainDelivery.deliver { completion(InputiaPermissionLifecycle.shared.epoch == permissionEpoch ? result : nil) }
    }
  }

  private static let typedCaptureQueue = DispatchQueue(label: "Inputia.typed-capture")
  /// 不启动服务；连接、握手和策略刷新仅发生在后台。过期事件丢弃不重放。
  static func typedCapture(_ command: InputiaTypedCaptureCommand, deadline: TimeInterval,
    expectedServer: String? = nil, completion: @escaping (InputiaTypedCaptureReply?) -> Void) {
    let permissionEpoch = InputiaPermissionLifecycle.shared.epoch
    typedCaptureQueue.async {
      var result: InputiaTypedCaptureReply?
      if ProcessInfo.processInfo.systemUptime < deadline {
        do {
          let connection = try reusableBusinessConnection(&typedCaptureConnection, epoch: permissionEpoch, expectedServer: expectedServer)
          if ProcessInfo.processInfo.systemUptime < deadline,
            expectedServer == nil || expectedServer == connection.server.instance_id {
            result = try connection.typedCapture(command)
          }
        } catch {
          typedCaptureConnection?.connection.closeTypedCaptureConnection(); typedCaptureConnection = nil
          // 不保留正文，不重试，不启动服务。
        }
      }
      let reply = result
      DispatchQueue.main.async { completion(InputiaPermissionLifecycle.shared.epoch == permissionEpoch ? reply : nil) }
    }
  }

  private static let targetQueue = DispatchQueue(label: "Inputia.target-broker")
  private static var permissionConnection: InputiaVoiceServiceConnection?
  private static var targetConnection: InputiaVoiceServiceConnection?
  static func probeServicePermission() -> Bool {
    ensureUnifiedServiceReady()
    let started = ProcessInfo.processInfo.systemUptime
    do {
      if permissionConnection == nil { permissionConnection = try openAuthenticatedConnection() }
      guard let connection = permissionConnection else { return false }
      let reply = try connection.targetBridge(.init(kind: "status"))
      if !reply.ready {
        InputiaPermissionLifecycle.shared.observeService(server: reply.server_instance, epoch: reply.permission_epoch,
          deadline: ProcessInfo.processInfo.systemUptime, ready: false)
        return false
      }
      guard ProcessInfo.processInfo.systemUptime - started < 0.75, ProcessInfo.processInfo.systemUptime < reply.deadline else { return false }
      InputiaPermissionLifecycle.shared.observeService(server: reply.server_instance, epoch: reply.permission_epoch,
        deadline: reply.deadline, ready: reply.ready)
      return reply.ready && ProcessInfo.processInfo.systemUptime < reply.deadline
    } catch { permissionConnection?.close(); permissionConnection = nil; return false }
  }
  static var didReleaseTarget: ((String) -> Void)?
  static func releaseTarget(_ id: String) {
    DispatchQueue.main.async { didReleaseTarget?(id) }
    targetBridge(.init(kind: "release", target_id: id)) { _ in }
  }
  static func targetBridge(_ command: InputiaTargetBridgeCommand,
    personalAdmissionDelivery: Bool = false,
    completion: @escaping (InputiaTargetBridgeReply?) -> Void) {
    let epoch = InputiaPermissionLifecycle.shared.epoch
    targetQueue.async {
      var result: InputiaTargetBridgeReply?
      do {
        guard command.kind == "release" ? InputiaPermissionLifecycle.shared.allowsServiceConnection : InputiaPermissionLifecycle.shared.permits(epoch) else { throw InputiaVoiceServiceError.policy }
        if targetConnection == nil { targetConnection = try openAuthenticatedConnection() }
        guard let connection = targetConnection else { throw InputiaVoiceServiceError.policy }
        let reply = try connection.targetBridge(command)
        if command.kind == "capture" || command.kind == "validate" {
          // flags: bit0 ready, bit1同服务租约, bit2权限epoch有效, bit3未过期。
          let flags = (reply.ready ? 1 : 0)
            | (InputiaPermissionLifecycle.shared.matchesService(server: reply.server_instance, epoch: reply.permission_epoch) ? 2 : 0)
            | (InputiaPermissionLifecycle.shared.permits(epoch) ? 4 : 0)
            | (ProcessInfo.processInfo.systemUptime < reply.deadline ? 8 : 0)
          InputiaPersonalizationDiagnostics.record(command.kind == "capture" ? "target_capture" : "target_validate", reply.code ?? "ok", flags: flags)
        }
        if reply.ready, InputiaPermissionLifecycle.shared.matchesService(server: reply.server_instance, epoch: reply.permission_epoch), InputiaPermissionLifecycle.shared.permits(epoch), ProcessInfo.processInfo.systemUptime < reply.deadline { result = reply }
        else if command.kind == "capture", let abandoned = reply.target {
          _ = try? connection.targetBridge(.init(kind: "release", target_id: abandoned.target_id))
        }
      } catch {
        InputiaPersonalizationDiagnostics.record("target_transport", InputiaPersonalizationDiagnostics.errorReason(error))
        targetConnection?.close(); targetConnection = nil
      }
      let value = result
      let deliver = {
        let allowed = InputiaPermissionLifecycle.shared.permits(epoch)
        if !allowed, command.kind == "capture", let abandoned = value?.target { releaseTarget(abandoned.target_id) }
        completion(allowed ? value : nil)
      }
      if personalAdmissionDelivery { InputiaPersonalMainDelivery.deliver(deliver) }
      else { DispatchQueue.main.async(execute: deliver) }
    }
  }

  /// 输入法启用时异步准备同一签名配对服务，仅隐藏启动，不发送任何录音命令。
  static func ensureUnifiedServiceReady() {
    readinessQueue.async {
      guard readiness.needsAutomaticPreparation, InputiaPermissionLifecycle.shared.allowsServiceConnection else { return }
      do {
        let profile = InputiaProfile.current
        try profile.validateCandidatePaths()
        let bytes = try profile.readPairManifest()
        let manifest = try SignedPairManifest.verify(bytes, trust: InputiaEmbeddedPairTrust.trust)
        let identity = try manifest.identity(for: .handy)
        if serviceTerminationObserver == nil {
          serviceTerminationObserver = NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didTerminateApplicationNotification, object: nil, queue: nil
          ) { notification in
            guard let app = notification.userInfo?[NSWorkspace.applicationUserInfoKey] as? NSRunningApplication,
              app.bundleIdentifier == identity.identifier else { return }
            readinessQueue.async { readiness.suspend() }
            retireBusinessConnections()
          }
        }
        let running = try trustedServiceIsRunning(identity: identity)
        if running { readiness.markServiceObserved(); return }
        guard readiness.requestStart(isRunning: false) else { return }
        let app = try verifiedInstalledService(identity: identity)
        guard InputiaPermissionLifecycle.shared.allowsServiceConnection,
          InputiaPermissionLifecycle.shared.backgroundMaintenanceAllowsWork() else { return }
        readinessLaunches.enter()
        openHiddenService(appPath: app.path) { error in
          defer { readinessLaunches.leave() }
          if error != nil { NSLog("inputia_service_prepare_failed automatic_retry=false") }
          else { NSLog("inputia_service_hidden_start_requested recording_command_sent=false") }
        }
      } catch {
        NSLog("inputia_service_prepare_unavailable automatic_retry=false")
      }
    }
  }

  /// 菜单关闭状态不能由旧端点的连接失败推断，必须核验当前配对身份。
  static func refreshServiceMenu(completion: @escaping (InputiaMenuReply?, InputiaVoiceServiceMenuState) -> Void) {
    menuAction(kind: "status") { snapshot in
      if let snapshot { completion(snapshot, .running); return }
      readinessQueue.async {
        var status = InputiaVoiceServiceMenuState.unavailable
        do {
          let identity = try verifiedServiceIdentity()
          status = try trustedServiceIsRunning(identity: identity) ? .running : .stopped
        } catch { NSLog("inputia_service_menu_unavailable") }
        let result = status
        DispatchQueue.main.async { completion(nil, result) }
      }
    }
  }

  private static func verifiedServiceIdentity() throws -> PairCodeIdentity {
    let profile = InputiaProfile.current
    try profile.validateCandidatePaths()
    let bytes = try profile.readPairManifest()
    guard bytes.count <= 16_384 else { throw InputiaVoiceServiceError.handshake }
    return try SignedPairManifest.verify(bytes, trust: InputiaEmbeddedPairTrust.trust).identity(for: .handy)
  }

  private static func trustedServiceIsRunning(identity: PairCodeIdentity) throws -> Bool {
    let apps = NSRunningApplication.runningApplications(withBundleIdentifier: identity.identifier)
    guard !apps.isEmpty else { return false }
    let hashes = identity.cdhashes.map { "cdhash H\"\($0)\"" }.joined(separator: " or ")
    var requirement: SecRequirement?
    guard SecRequirementCreateWithString("identifier \"\(identity.identifier)\" and (\(hashes))" as CFString, [], &requirement) == errSecSuccess,
      let requirement else { throw InputiaVoiceServiceError.handshake }
    for app in apps {
      if let installation = InputiaProfile.current.installation {
        guard app.bundleURL?.path == installation.receipt.components.control else {
          throw InputiaVoiceServiceError.profile
        }
      }
      var code: SecCode?
      let attributes = [kSecGuestAttributePid as String: NSNumber(value: app.processIdentifier)] as CFDictionary
      guard SecCodeCopyGuestWithAttributes(nil, attributes, [], &code) == errSecSuccess, let code,
        SecCodeCheckValidity(code, [], requirement) == errSecSuccess else { throw InputiaVoiceServiceError.handshake }
    }
    return true
  }

  /// 只有显式打开能解除退出后的暂停；使用原有签名验证和隐藏启动路径。
  static func startUnifiedService(completion: @escaping (Bool) -> Void) {
    readinessQueue.async {
      guard !explicitStartPending else { DispatchQueue.main.async { completion(false) }; return }
      explicitStartPending = true
      do {
        let identity = try verifiedServiceIdentity()
        let app = try verifiedInstalledService(identity: identity)
        guard InputiaPermissionLifecycle.shared.allowsServiceConnection,
          InputiaPermissionLifecycle.shared.backgroundMaintenanceAllowsWork() else { throw InputiaVoiceServiceError.policy }
        let running = try trustedServiceIsRunning(identity: identity)
        readiness.resumeForExplicitStart()
        if running { readiness.markServiceObserved(); explicitStartPending = false; DispatchQueue.main.async { completion(true) }; return }
        guard readiness.requestStart(isRunning: false) else { explicitStartPending = false; DispatchQueue.main.async { completion(false) }; return }
        readinessLaunches.enter()
        openHiddenService(appPath: app.path) { error in
          readinessQueue.async {
            readinessLaunches.leave()
            explicitStartPending = false
            DispatchQueue.main.async { completion(error == nil) }
          }
        }
      } catch {
        explicitStartPending = false
        NSLog("inputia_service_explicit_start_unavailable")
        DispatchQueue.main.async { completion(false) }
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
  private struct Endpoint: Decodable { let profile_id: String; let protocol_major: Int; let server_instance: String; let socket_path: String; let pair_binding: InputiaPairBinding? }

  /// All IPC ownership is released on its owning queues. Never wait for a socket on main.
  static func invalidatePermissionWork(completion: @escaping () -> Void = {}) {
    InputiaSharedTermsMemory.shared.clear()
    let retired = DispatchGroup()
    retired.enter()
    personalizationQueue.async {
      personalizationConnection?.connection.closeTypedCaptureConnection(); personalizationConnection = nil
      retired.leave()
    }
    retired.enter()
    typedCaptureQueue.async {
      typedCaptureConnection?.connection.closeTypedCaptureConnection(); typedCaptureConnection = nil
      retired.leave()
    }
    retired.enter()
    shortcutQueue.async {
      defer { retired.leave() }
      shortcutTimer?.cancel(); shortcutTimer = nil
      shortcutConnection?.close(); shortcutConnection = nil
      shortcutLease = nil; shortcutLeaseEpoch &+= 1
      shortcutHasActiveOwner = false; shortcutProviderHadTarget = nil
      shortcutCycleBusy = false
    }
    retired.enter()
    sharedTermsQueue.async {
      defer { retired.leave() }
      sharedTermsConnection?.close(); sharedTermsConnection = nil
    }
    retired.enter()
    voiceQueue.async {
      defer { retired.leave() }
      unifiedConnection?.close(); unifiedConnection = nil; unifiedSession = nil
      shortcutSession = nil; shortcutTarget = nil; shortcutServer = nil; shortcutTerms = nil
      shortcutDeliver = nil; shortcutStatus = nil
      lastUnifiedPhase = nil
      unifiedFetchGate = InputiaVoiceFirstFetchGate()
    }
    // Include a launch/verification operation that was already queued before the gate closed.
    retired.enter()
    readinessQueue.async { readinessLaunches.notify(queue: readinessQueue) { retired.leave() } }
    retired.enter()
    InputiaPermissionLifecycle.shared.retireProbe {
      permissionConnection?.close(); permissionConnection = nil; retired.leave()
    }
    retired.enter()
    targetQueue.async { targetConnection?.close(); targetConnection = nil; retired.leave() }
    retired.notify(queue: .main, execute: completion)
  }

  /// provider 和新会话复核在主线程；定时器、认证、数据库和 socket 均在后台。
  static func startShortcutListening(
    targetProvider: @escaping () -> InputiaVoiceTarget?,
    sharedTermsReceiver: @escaping (InputiaSharedTermsSnapshot, UInt64) -> Void,
    acceptStart: @escaping (InputiaHostShortcutTrigger, @escaping (Bool) -> Void) -> Void
  ) {
    let permissionEpoch = InputiaPermissionLifecycle.shared.epoch
    shortcutQueue.async {
      guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else { return }
      guard shortcutTimer == nil else { return }
      let timer = DispatchSource.makeTimerSource(queue: shortcutQueue)
      timer.schedule(deadline: .now(), repeating: .milliseconds(200), leeway: .milliseconds(30))
      timer.setEventHandler {
        guard InputiaPermissionLifecycle.shared.permits(permissionEpoch), !shortcutCycleBusy, ProcessInfo.processInfo.systemUptime >= shortcutRetryAfter else { return }
        shortcutCycleBusy = true
        DispatchQueue.main.async {
          guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else {
            shortcutQueue.async { shortcutCycleBusy = false }; return
          }
          let target = targetProvider()
          if target == nil { InputiaSharedTermsMemory.shared.clear() }
          shortcutQueue.async {
            guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else { shortcutCycleBusy = false; return }
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
                  guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else { throw InputiaVoiceServiceError.policy }
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
                        guard InputiaPermissionLifecycle.shared.permits(permissionEpoch), InputiaSharedTermsMemory.shared.ticket() == ticket else { return }
                        guard let termsConnection = sharedTermsConnection,
                          InputiaVoiceServiceConnection.sharedTermsConnectionMatches(
                            server: termsConnection.server.instance_id, primaryServer: server,
                            version: termsConnection.locallyAppliedVersion, primaryVersion: version)
                        else { throw InputiaVoiceServiceError.policy }
                        let validation = try termsConnection.targetBridge(.init(kind: "validate", target: renewed.target, purpose: "shared_terms"))
                        guard validation.ready, InputiaPermissionLifecycle.shared.matchesService(server: validation.server_instance, epoch: validation.permission_epoch),
                          ProcessInfo.processInfo.systemUptime < validation.deadline else { return }
                        if let snapshot = try termsConnection.fetchSharedTerms(lease: renewed, leaseDeadline: min(leaseDeadline, validation.deadline)) {
                          DispatchQueue.main.async {
                            guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else { return }
                            sharedTermsReceiver(snapshot, ticket)
                          }
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
                  guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else {
                    shortcutCycleBusy = false; return
                  }
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
                DispatchQueue.main.async {
                  guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else { finished(false); return }
                  acceptStart(trigger, finished)
                }
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

  static func openAuthenticatedConnection() throws -> InputiaVoiceServiceConnection {
    guard InputiaPermissionLifecycle.shared.allowsServiceConnection, InputiaPermissionLifecycle.shared.backgroundMaintenanceAllowsWork() else { throw InputiaVoiceServiceError.policy }
    let profile = InputiaProfile.current
    try profile.validateCandidatePaths()
    let endpoint = try JSONDecoder().decode(Endpoint.self,
      from: profile.readEndpoint())
    guard endpoint.protocol_major == 1,
      endpoint.profile_id == profile.profileID, endpoint.pair_binding == profile.pairBinding else { throw InputiaVoiceServiceError.profile }
    let manifest = try profile.readPairManifest()
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
    if let target = shortcutTarget { releaseTarget(target.target_id) }
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
    let permissionEpoch = InputiaPermissionLifecycle.shared.epoch
    voiceQueue.async {
      guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else { completion(false); return }
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
        guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else { throw InputiaVoiceServiceError.policy }
        if trigger.starts_session {
          let validation = try connection.targetBridge(.init(kind: "validate", target: trigger.target, purpose: "start"))
          guard validation.ready, InputiaPermissionLifecycle.shared.matchesService(server: validation.server_instance, epoch: validation.permission_epoch),
            ProcessInfo.processInfo.systemUptime < validation.deadline else { throw InputiaVoiceServiceError.policy }
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

  /// 退休旧菜单连接和会话归属；已提取结果由独立回执闭包结束，不重放。
  private static func retireMenuServiceConnection() {
    retireBusinessConnections()
    let oldSession = unifiedSession
    let oldConnection = unifiedConnection
    let hasPendingReceipt = unifiedFetchGate.attempted
    unifiedConnection = nil
    unifiedSession = nil
    lastUnifiedPhase = nil
    if !hasPendingReceipt { oldConnection?.close() }
    if let owner = shortcutSession, oldSession == nil || owner == oldSession {
      clearShortcutOwnership(sessionID: owner)
    }
  }

  /// 统一菜单与语音共用串行通讯队列；不在系统菜单或按键回调等待服务。
  static func menuAction(kind: String, modelID: String? = nil, completion: @escaping (InputiaMenuReply?) -> Void) {
    if kind == "start_service" {
      startUnifiedService { started in
        if started { refreshServiceMenu { reply, _ in completion(reply) } }
        else { completion(nil) }
      }
      return
    }
    if kind == "quit_service" { readinessQueue.async { readiness.suspend() } }
    voiceQueue.async {
      do {
        if let connection = unifiedConnection {
          guard activeSessionMenuActions.contains(kind) else {
            DispatchQueue.main.async { completion(nil) }; return
          }
          let reply = try connection.menuRequest(kind: kind, modelID: modelID)
          if kind == "quit_service", reply.status == "menu" { retireMenuServiceConnection() }
          DispatchQueue.main.async { completion(reply.status == "menu" ? reply : nil) }
          return
        }
        let profile = InputiaProfile.current
        try profile.validateCandidatePaths()
        let endpoint = try JSONDecoder().decode(Endpoint.self,
          from: profile.readEndpoint())
        guard endpoint.protocol_major == 1,
          endpoint.profile_id == profile.profileID, endpoint.pair_binding == profile.pairBinding else { throw InputiaVoiceServiceError.profile }
        let manifest = try profile.readPairManifest()
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
        retireMenuServiceConnection()
        NSLog("inputia_menu_request_unconfirmed automatic_replay=false")
        DispatchQueue.main.async { completion(nil) }
      }
    }
  }

  /// 实际菜单仅入队，绝不在InputMethodKit主线程等待IPC或数据库。
  static func triggerUnifiedVoice(target: InputiaVoiceTarget?,
    deliver: @escaping (InputiaVoiceDelivery, @escaping (String) -> Void) -> Void,
    completion: @escaping (String) -> Void) {
    let permissionEpoch = InputiaPermissionLifecycle.shared.epoch
    voiceQueue.async {
      var retainedTarget = false
      defer { if !retainedTarget, let target { releaseTarget(target.target_id) } }
      guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else {
        DispatchQueue.main.async { completion("输入法权限不可用，语音已暂停。") }; return
      }
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
          from: profile.readEndpoint())
        guard endpoint.profile_id == profile.profileID, endpoint.pair_binding == profile.pairBinding, endpoint.protocol_major == 1 else { throw InputiaVoiceServiceError.profile }
        stage = "read_manifest"
        let manifest = try InputiaProfile.current.readPairManifest()
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
        guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else { connection.close(); throw InputiaVoiceServiceError.policy }
        stage = "start_request"
        let validation = try connection.targetBridge(.init(kind: "validate", target: target, purpose: "start"))
        guard validation.ready, InputiaPermissionLifecycle.shared.matchesService(server: validation.server_instance, epoch: validation.permission_epoch),
          ProcessInfo.processInfo.systemUptime < validation.deadline else { connection.close(); throw InputiaVoiceServiceError.policy }
        let reply = try connection.request(sessionID: session, requestID: UUID().uuidString,
          command: .start(target: target, postProcess: false, terms: version))
        guard reply.status == "session" else { connection.close(); throw InputiaVoiceServiceError.handshake }
        unifiedConnection = connection
        unifiedSession = session
        retainedTarget = true
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
    let permissionEpoch = InputiaPermissionLifecycle.shared.epoch
    voiceQueue.asyncAfter(deadline: .now() + 0.25) {
      guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else { return }
      guard unifiedSession == session, unifiedConnection === connection, !unifiedFetchGate.attempted else {
        if unifiedSession != session { releaseTarget(target.target_id) }
        return
      }
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
            releaseTarget(target.target_id)
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
          guard InputiaPermissionLifecycle.shared.permits(permissionEpoch) else { outputConnection.close(); return }
          guard var delivery = try outputConnection.fetchDelivery(view: refreshed.1, target: target) else {
            releaseTarget(target.target_id)
            outputConnection.close(); unifiedConnection = nil; unifiedSession = nil
            clearShortcutOwnership(sessionID: session)
            DispatchQueue.main.async { completion("结果已有输出状态，未重复插入。请在 Inputia 查看历史。") }
            return
          }
          let permit = try outputConnection.targetBridge(.init(kind: "validate", target: target,
            purpose: "dispatch", operation_id: delivery.operation_id))
          guard permit.ready, let nonce = permit.dispatch_nonce, !nonce.isEmpty,
            InputiaPermissionLifecycle.shared.matchesService(server: permit.server_instance, epoch: permit.permission_epoch),
            ProcessInfo.processInfo.systemUptime < permit.deadline else {
            try outputConnection.acknowledgeDelivery(delivery, receipt: "pending_target")
            throw InputiaVoiceServiceError.policy
          }
          delivery.dispatchNonce = nonce
          delivery.dispatchDeadline = min(delivery.dispatchDeadline, permit.deadline)
          let permittedDelivery = delivery
          let receiptGate = InputiaVoiceReceiptGate()
          DispatchQueue.main.async {
            let delivery = permittedDelivery
            let acknowledge: (String) -> Void = { receipt in
              voiceQueue.async {
                guard receiptGate.claim() else { return }
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
                releaseTarget(target.target_id)
                outputConnection.close()
                if unifiedSession == session, unifiedConnection === outputConnection {
                  unifiedConnection = nil; unifiedSession = nil
                }
                clearShortcutOwnership(sessionID: session)
              }
            }
            guard InputiaPermissionLifecycle.shared.permits(permissionEpoch), ProcessInfo.processInfo.systemUptime < delivery.dispatchDeadline else {
              acknowledge("pending_target"); return
            }
            deliver(delivery, acknowledge)
          }
        } else {
          releaseTarget(target.target_id)
          NSLog("inputia_unified_voice_session_terminal phase=%@", view.phase)
          connection.close(); unifiedConnection = nil; unifiedSession = nil
          clearShortcutOwnership(sessionID: session)
          DispatchQueue.main.async { completion(view.phase == "pending_target" ? "转写已保存到历史记录，结果待插入。" : "语音会话已结束。") }
        }
      } catch {
        releaseTarget(target.target_id)
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

  static func checkMenuConnectionRetirement() -> Bool {
    unifiedSession = "synthetic-menu-session"
    shortcutSession = "synthetic-menu-session"
    shortcutServer = "synthetic-menu-server"
    lastUnifiedPhase = "recording"
    retireMenuServiceConnection()
    return unifiedSession == nil && unifiedConnection == nil && shortcutSession == nil
      && shortcutServer == nil && lastUnifiedPhase == nil
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
    #if INPUTIA_RELEASE_PAIR_V2
    guard let installation = InputiaProfile.current.installation else { return [] }
    return [installation.receipt.components.control]
    #else
    return ["/Applications/Inputia.app", "\(homeDirectory)/Applications/Inputia.app",
      "/Applications/Inputia Candidate.app", "\(homeDirectory)/Applications/Inputia Candidate.app"]
    #endif
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
