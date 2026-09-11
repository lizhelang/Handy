import Darwin
import Foundation

@main
struct InputiaVoiceInputLauncherSelfCheck {
  static func main() {
    #if INPUTIA_PAIRED_BUILD && INPUTIA_CONNECTION_SELF_CHECK
    precondition(InputiaVoiceInputLauncher.checkPreFetchFailureReleasesWait())
    print("preFetchFailureReleasesWait=true shortcut_owner_retained=true")
    #endif
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
    var observedExit = InputiaVoiceServiceReadiness()
    precondition(!observedExit.requestStart(isRunning: true))
    observedExit.suspend()
    precondition(!observedExit.requestStart(isRunning: false))
    precondition(InputiaVoiceInputLauncher.installedServiceAppPaths(homeDirectory: "/Users/example") == [
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
}
