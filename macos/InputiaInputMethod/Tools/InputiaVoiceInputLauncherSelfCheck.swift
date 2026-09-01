import Darwin
import Foundation

@main
struct InputiaVoiceInputLauncherSelfCheck {
  static func main() {
    let fakeApp = "/tmp/InputiaVoiceInputLauncherSelfCheck/Handy.app"
    let fakeExecutable = "\(fakeApp)/Contents/MacOS/Handy"
    let environment = ["INPUTIA_HANDY_APP": fakeApp]
    let expectedCandidates = InputiaVoiceInputLauncher.candidateAppPaths(
      environment: environment,
      homeDirectory: "/Users/example",
      workspaceAppPath: "/Applications/Handy.app"
    )
    let candidatesAreOrdered = expectedCandidates.prefix(3) == [
      fakeApp,
      "/Applications/Handy.app",
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
    print("candidatesAreOrdered=\(candidatesAreOrdered)")
    print("findsEnvApp=\(findsEnvApp)")
    print("missingWhenExecutableAbsent=\(missingWhenExecutableAbsent)")
    print("runningPlanTogglesImmediately=\(runningPlanTogglesImmediately)")
    print("coldPlanStartsHiddenThenToggles=\(coldPlanStartsHiddenThenToggles)")
    exit(ok ? 0 : 1)
  }
}
