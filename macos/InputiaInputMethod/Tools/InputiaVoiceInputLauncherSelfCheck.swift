import Darwin
import Foundation

@main
struct InputiaVoiceInputLauncherSelfCheck {
  static func main() {
    checkConnectionRetryGate()
    #if INPUTIA_PAIRED_BUILD && INPUTIA_CONNECTION_SELF_CHECK
    precondition(InputiaVoiceInputLauncher.checkPreFetchFailureReleasesWait())
    precondition(InputiaVoiceInputLauncher.checkMenuConnectionRetirement())
    print("menuServiceRetirementClearsStaleOwnership=true")
    print("preFetchFailureReleasesWait=true shortcut_owner_retained=true")
    #endif
    let receipt = InputiaVoiceReceiptGate()
    precondition(receipt.claim())
    precondition(!receipt.claim())
    print("retiredSessionReceiptRemainsSingleUse=true")
    let endpoint = Data([7, 8])
    let scope = InputiaAuthenticatedConnectionScope(permissionEpoch: 2, endpoint: endpoint,
      manifest: Data([1, 2]), server: "trusted-server")
    precondition(scope.matches(epoch: 2, endpoint: endpoint, manifest: Data([1, 2]),
      expectedServer: "trusted-server"))
    precondition(!scope.matches(epoch: 3, endpoint: endpoint, manifest: Data([1, 2]),
      expectedServer: "trusted-server"))
    precondition(!scope.matches(epoch: 2, endpoint: endpoint, manifest: Data([2, 3]),
      expectedServer: "trusted-server"))
    precondition(!scope.matches(epoch: 2, endpoint: Data([8, 9]), manifest: Data([1, 2]),
      expectedServer: "trusted-server"))
    precondition(!scope.matches(epoch: 2, endpoint: endpoint, manifest: Data([1, 2]),
      expectedServer: "replacement-server"))
    var observedRunning = InputiaVoiceServiceReadiness()
    precondition(observedRunning.needsAutomaticPreparation)
    observedRunning.markServiceObserved()
    precondition(!observedRunning.needsAutomaticPreparation)
    observedRunning.resumeForExplicitStart()
    precondition(observedRunning.needsAutomaticPreparation)
    print("authenticatedConnectionScopeInvalidates=true readinessAuditOnce=true")
    var firstFetch = InputiaVoiceFirstFetchGate()
    precondition(!firstFetch.claimAfterPolicyRefresh(verified: false))
    precondition(!firstFetch.attempted)
    precondition(firstFetch.claimAfterPolicyRefresh(verified: true))
    precondition(firstFetch.attempted)
    precondition(!firstFetch.claimAfterPolicyRefresh(verified: true))
    var nextSessionFetch = InputiaVoiceFirstFetchGate()
    precondition(!nextSessionFetch.claimAfterPolicyRefresh(verified: false))
    precondition(nextSessionFetch.claimAfterPolicyRefresh(verified: true))
    precondition(InputiaVoiceInputLauncher.activeSessionMenuActions.contains("quit_service"))
    precondition(!InputiaVoiceInputLauncher.activeSessionMenuActions.contains("select_model"))
    var readiness = InputiaVoiceServiceReadiness()
    precondition(!readiness.requestStart(isRunning: true))
    precondition(readiness.requestStart(isRunning: false))
    precondition(!readiness.requestStart(isRunning: false))
    readiness.suspend()
    precondition(!readiness.requestStart(isRunning: false))
    var explicitlyQuit = InputiaVoiceServiceReadiness()
    explicitlyQuit.suspend()
    precondition(!explicitlyQuit.requestStart(isRunning: false))
    explicitlyQuit.resumeForExplicitStart()
    precondition(explicitlyQuit.requestStart(isRunning: false))
    precondition(!explicitlyQuit.requestStart(isRunning: false))
    precondition(InputiaVoiceServiceMenuState.stopped.action == "start_service")
    precondition(InputiaVoiceServiceMenuState.running.action == "quit_service")
    precondition(InputiaVoiceServiceMenuState.unavailable.action == nil)
    precondition(InputiaVoiceInputLauncher.startupArguments == ["--start-hidden"])
    print("explicitServiceRestartAfterQuit=true trustedMenuLifecycle=true")
    var observedExit = InputiaVoiceServiceReadiness()
    precondition(!observedExit.requestStart(isRunning: true))
    observedExit.suspend()
    precondition(!observedExit.requestStart(isRunning: false))
    precondition(InputiaVoiceInputLauncher.installedServiceAppPaths(homeDirectory: "/Users/example") == [
      "/Applications/Inputia.app", "/Users/example/Applications/Inputia.app",
      "/Applications/Inputia Candidate.app", "/Users/example/Applications/Inputia Candidate.app",
    ])
    let fakeApp = "/tmp/InputiaVoiceInputLauncherSelfCheck/Inputia.app"
    let fakeExecutable = "\(fakeApp)/Contents/MacOS/handy"
    try? FileManager.default.removeItem(atPath: "/tmp/InputiaVoiceInputLauncherSelfCheck")
    try? FileManager.default.createDirectory(
      atPath: "\(fakeApp)/Contents/MacOS",
      withIntermediateDirectories: true
    )
    FileManager.default.createFile(atPath: fakeExecutable, contents: Data())
    defer { try? FileManager.default.removeItem(atPath: "/tmp/InputiaVoiceInputLauncherSelfCheck") }

    let environment = ["INPUTIA_HANDY_APP": fakeApp]
    let expectedCandidates = InputiaVoiceInputLauncher.candidateAppPaths(
      environment: environment,
      homeDirectory: "/Users/example",
      workspaceAppPath: "/Applications/Inputia.app",
      legacyWorkspaceAppPath: "/Applications/Handy.app"
    )
    let candidatesAreOrdered = expectedCandidates.prefix(5) == [
      fakeApp,
      "/Applications/Inputia.app",
      "/Applications/Handy.app",
      "/Users/example/Applications/Inputia.app",
      "/Users/example/Applications/Handy.app",
    ]

    let findsEnvApp = InputiaVoiceInputLauncher.findHandyApp(
      environment: environment,
      homeDirectory: "/Users/example",
      workspaceAppPath: nil,
      fileExists: { $0 == fakeExecutable }
    ) == fakeApp

    let missingWhenExecutableAbsent = InputiaVoiceInputLauncher.findHandyApp(
      environment: environment,
      homeDirectory: "/Users/example",
      workspaceAppPath: nil,
      fileExists: { _ in false }
    ) == nil

    let runningPlan = InputiaVoiceInputLauncher.launchPlan(appPath: fakeApp, isRunning: true)
    let coldPlan = InputiaVoiceInputLauncher.launchPlan(appPath: fakeApp, isRunning: false)

    let runningPlanTogglesImmediately = !runningPlan.delayed
      && runningPlan.toggleArguments == ["--toggle-transcription"]
      && runningPlan.executablePath == fakeExecutable

    let coldPlanStartsHiddenThenToggles = coldPlan.delayed
      && coldPlan.startupArguments == ["--start-hidden"]
      && coldPlan.toggleArguments == ["--toggle-transcription"]
      && coldPlan.toggleDelaySeconds > 0

    let ok = candidatesAreOrdered
      && findsEnvApp
      && missingWhenExecutableAbsent
      && runningPlanTogglesImmediately
      && coldPlanStartsHiddenThenToggles

    print("voiceInputLauncherSelfCheck=\(ok)")
    print("voiceFirstFetchGateSelfCheck=true refresh_required=true once_per_session=true unknown_fetch_not_replayed=true new_session_not_blocked=true")
    print("serviceReadinessSelfCheck=true cold_start_once=true explicit_quit_suppressed=true observed_exit_suppressed=true installed_paths_only=true")
    print("candidatesAreOrdered=\(candidatesAreOrdered)")
    print("findsEnvApp=\(findsEnvApp)")
    print("missingWhenExecutableAbsent=\(missingWhenExecutableAbsent)")
    print("runningPlanTogglesImmediately=\(runningPlanTogglesImmediately)")
    print("coldPlanStartsHiddenThenToggles=\(coldPlanStartsHiddenThenToggles)")
    exit(ok ? 0 : 1)
  }

  private static func checkConnectionRetryGate() {
    let gate = InputiaConnectionRetryGate()
    var attempts = 0
    // 六个后台入口共享同一退避；推进合成时钟，不等待30秒或读取真实用户资料。
    for tick in 0..<120 {
      let now = Double(tick) * 0.25
      for _ in 0..<6 {
        guard let attempt = gate.begin(now: now) else { continue }
        attempts += 1
        precondition(gate.begin(now: now) == nil, "并发入口不能重复启动审计")
        gate.finish(attempt, succeeded: false, now: now)
      }
    }
    precondition(attempts == 6, "失败审计必须按1/2/4/8秒退避且所有入口共享")
    let recovered = gate.begin(now: 31)
    precondition(recovered != nil, "稳定失败必须在最长八秒后再次尝试")
    gate.finish(recovered!, succeeded: true, now: 31)
    let healthy = gate.begin(now: 31)
    precondition(healthy != nil, "成功连接不能被失败冷却限速")
    gate.finish(healthy!, succeeded: false, now: 31)
    gate.resumeForExplicitStart()
    let explicit = gate.begin(now: 31)
    precondition(explicit != nil, "显式启动必须可立即恢复")
    gate.resumeForExplicitStart()
    precondition(gate.begin(now: 31) == nil, "恢复不能取消仍在执行的连接")
    gate.finish(explicit!, succeeded: false, now: 31)
    let afterOldFailure = gate.begin(now: 31)
    precondition(afterOldFailure != nil, "旧请求失败不能覆盖显式恢复")
    gate.finish(afterOldFailure!, succeeded: true, now: 31)

    let concurrent = InputiaConnectionRetryGate()
    let lock = NSLock()
    var winners = 0
    var held: InputiaConnectionRetryGate.Attempt?
    DispatchQueue.concurrentPerform(iterations: 64) { _ in
      if let attempt = concurrent.begin(now: 0) {
        lock.lock(); winners += 1; held = attempt; lock.unlock()
      }
    }
    precondition(winners == 1, "跨队列同时连接必须只允许一个审计")
    concurrent.finish(held!, succeeded: false, now: 0)
    precondition(concurrent.begin(now: 0.99) == nil)
    precondition(concurrent.begin(now: 1) != nil)
    print("connectionFailureBackoff=true simulated_seconds=30 callers=6 requests=720 audit_attempts=\(attempts) max_retry_seconds=8 explicitRecovery=true concurrentSingleflight=true")
  }
}
