import Darwin
import Foundation

@main
struct InputiaInputTextRouterSelfCheck {
  private static func emit(_ line: String) {
    FileHandle.standardOutput.write(Data((line + "\n").utf8))
  }

  private static func route(_ text: String, bridge: InputiaRustBridge) -> [InputiaBridgeOutcome] {
    var outcomes: [InputiaBridgeOutcome] = []
    var hasComposing = false

    for character in text {
      let outcome: InputiaBridgeOutcome
      switch InputiaInputTextRouter.action(for: character, hasComposing: hasComposing) {
      case .passthrough:
        continue
      case .enter:
        outcome = bridge.enter()
      case .space:
        outcome = bridge.space()
      case .character(let routedCharacter):
        outcome = bridge.handle(character: routedCharacter)
      }
      outcomes.append(outcome)
      hasComposing = !outcome.composing.isEmpty
    }

    return outcomes
  }

  private static func routeInChineseMode(_ text: String) -> [InputiaBridgeOutcome] {
    let bridge = InputiaRustBridge.temporarySettingsForDiagnostics()
    _ = bridge.setChineseMode()
    return route(text, bridge: bridge)
  }

  private static func commitInChineseMode(_ text: String, bridge: InputiaRustBridge) -> String? {
    _ = bridge.setChineseMode()
    return route(text, bridge: bridge).last?.commit
  }

  static func main() {
    let carriageReturnOutcomes = routeInChineseMode("ni\r")
    let lineFeedOutcomes = routeInChineseMode("ni\n")
    let spaceOutcomes = routeInChineseMode("ni ")
    let plainSpaceOutcomes = routeInChineseMode(" ")
    let simplifiedBridge = InputiaRustBridge.temporarySettingsForDiagnostics(chineseScript: "simplified")
    let traditionalBridge = InputiaRustBridge.temporarySettingsForDiagnostics(chineseScript: "traditional")
    let simplifiedCommit = commitInChineseMode("zhongguo ", bridge: simplifiedBridge)
    let traditionalCommit = commitInChineseMode("zhongguo ", bridge: traditionalBridge)

    let carriageReturnCommitsRaw = carriageReturnOutcomes.last?.commit == "ni"
      && carriageReturnOutcomes.last?.composing == ""
    let lineFeedCommitsRaw = lineFeedOutcomes.last?.commit == "ni"
      && lineFeedOutcomes.last?.composing == ""
    let composingSpaceCommitsCandidate = spaceOutcomes.last?.commit == "你"
      && spaceOutcomes.last?.composing == ""
    let plainSpacePassesThrough = plainSpaceOutcomes.last?.consumed == false
      && plainSpaceOutcomes.last?.commit == nil

    let routeEnterAction = InputiaInputTextRouter.action(for: "\r", hasComposing: true) == .enter
    let routeEnterPassesThroughWithoutComposing = InputiaInputTextRouter.action(
      for: "\r",
      hasComposing: false
    ) == .passthrough
    let routeSpaceAction = InputiaInputTextRouter.action(for: " ", hasComposing: true) == .space
    let routePlainSpaceAction = InputiaInputTextRouter.action(for: " ", hasComposing: false) == .character(" ")
    let routeLetterAction = InputiaInputTextRouter.action(for: "n", hasComposing: false) == .character("n")
    let simplifiedScriptCommitsSimplified = simplifiedCommit == "中国"
    let traditionalScriptCommitsTraditional = traditionalCommit == "中國"

    let ok = carriageReturnCommitsRaw
      && lineFeedCommitsRaw
      && composingSpaceCommitsCandidate
      && plainSpacePassesThrough
      && routeEnterAction
      && routeEnterPassesThroughWithoutComposing
      && routeSpaceAction
      && routePlainSpaceAction
      && routeLetterAction
      && simplifiedScriptCommitsSimplified
      && traditionalScriptCommitsTraditional

    emit("inputTextRouterSelfCheck=\(ok)")
    emit("carriageReturnCommitsRaw=\(carriageReturnCommitsRaw)")
    emit("lineFeedCommitsRaw=\(lineFeedCommitsRaw)")
    emit("composingSpaceCommitsCandidate=\(composingSpaceCommitsCandidate)")
    emit("plainSpacePassesThrough=\(plainSpacePassesThrough)")
    emit("routeEnterAction=\(routeEnterAction)")
    emit("routeEnterPassesThroughWithoutComposing=\(routeEnterPassesThroughWithoutComposing)")
    emit("routeSpaceAction=\(routeSpaceAction)")
    emit("routePlainSpaceAction=\(routePlainSpaceAction)")
    emit("routeLetterAction=\(routeLetterAction)")
    emit("simplifiedScriptCommit=\(simplifiedCommit ?? "nil")")
    emit("simplifiedScriptCommitsSimplified=\(simplifiedScriptCommitsSimplified)")
    emit("traditionalScriptCommit=\(traditionalCommit ?? "nil")")
    emit("traditionalScriptCommitsTraditional=\(traditionalScriptCommitsTraditional)")
    _exit(ok ? 0 : 1)
  }
}
