import Foundation

enum InputiaPersonalizationDiagnostics {
  static func record(_ phase: String, _ reason: String, flags: Int = 0, count: Int = 0) {}
}

// 隔离的合成传输：只替换socket，不替换实际个性化状态机；不访问权限或用户输入。
struct InputiaVoiceTarget: Equatable {
  let target_id: String
  var field_id: String? = "field"
}
struct InputiaPersonalCandidate: Equatable { let id: String; let text: String; let base_rank: Int; let consumed_len: Int }
struct InputiaPersonalPrediction: Equatable { let id: String; let text: String }
struct InputiaPersonalResult {
  let ordered_ids: [String]?; let predictions: [InputiaPersonalPrediction]?; let context_id: String?
  var admitted: Bool? = nil; var prediction_id: String? = nil
}
struct InputiaPersonalReply {
  let server_instance: String; let enabled: Bool; let epoch: UInt64
  let result: InputiaPersonalResult?; let code: String?
}
struct InputiaPersonalCommand {
  let kind: String
  var target: InputiaVoiceTarget? = nil
  var learning_epoch: UInt64? = nil
  var input_code: String? = nil
  var context: String? = nil
  var context_id: String? = nil
  var candidates: [InputiaPersonalCandidate]? = nil
  var limit: Int? = nil
  var event_id: String? = nil
  var text: String? = nil
  var previous: String? = nil
  var explicit_selection: Bool? = nil
  var original_rank: Int? = nil
  var operation: String? = nil
  var prediction_id: String? = nil
}
enum InputiaVoiceInputLauncher {
  struct Pending { let command: InputiaPersonalCommand; let completion: (InputiaPersonalReply?) -> Void }
  static var pending: [Pending] = []
  static func personalization(_ command: InputiaPersonalCommand, deadline: TimeInterval,
    expectedServer: String? = nil, completion: @escaping (InputiaPersonalReply?) -> Void) {
    pending.append(Pending(command: command, completion: completion))
  }
}
@main
struct PersonalizationFlowSelfCheck {
  static func drain(_ seconds: Double = 0.16) { RunLoop.main.run(until: Date().addingTimeInterval(seconds)) }
  static func reply(_ target: String = "origin-A", epoch: UInt64 = 1, enabled: Bool = true) -> InputiaPersonalReply {
    InputiaPersonalReply(server_instance: "service", enabled: enabled, epoch: epoch,
      result: InputiaPersonalResult(ordered_ids: ["candidate"], predictions: [], context_id: target), code: nil)
  }
  static func main() {
    let subject = InputiaPersonalization()
    subject.start()
    precondition(InputiaVoiceInputLauncher.pending.count == 1)
    InputiaVoiceInputLauncher.pending.removeFirst().completion(reply())
    let target = InputiaVoiceTarget(target_id: "origin-A")
    let candidates = [InputiaPersonalCandidate(id: "candidate", text: "你好", base_rank: 0, consumed_len: 5)]
    var installed: [String] = []
    subject.query(target: target, code: "n", candidates: candidates) { installed.append($0.code) }
    drain()
    precondition(InputiaVoiceInputLauncher.pending.count == 1)
    let old = InputiaVoiceInputLauncher.pending.removeFirst()
    subject.query(target: target, code: "ni", candidates: candidates) { installed.append($0.code) }
    drain()
    subject.query(target: target, code: "nihao", candidates: candidates) { installed.append($0.code) }
    drain()
    precondition(InputiaVoiceInputLauncher.pending.isEmpty)
    old.completion(reply())
    drain(0.02)
    precondition(installed.isEmpty)
    precondition(InputiaVoiceInputLauncher.pending.count == 1)
    let latest = InputiaVoiceInputLauncher.pending.removeFirst()
    precondition(latest.command.input_code == "nihao")
    latest.completion(reply())
    precondition(installed == ["nihao"])
    subject.query(target: target, code: "next", candidates: candidates) { installed.append($0.code) }
    drain()
    let foreign = InputiaVoiceInputLauncher.pending.removeFirst()
    subject.bind(InputiaVoiceTarget(target_id: "origin-B"))
    foreign.completion(reply())
    precondition(installed == ["nihao"] && subject.context.text.isEmpty)
    subject.bind(target)
    var predicted: InputiaPersonalization.View?
    subject.query(target: target, code: "", candidates: []) { predicted = $0 }
    drain()
    let predictionQuery = InputiaVoiceInputLauncher.pending.removeFirst()
    let prediction = InputiaPersonalPrediction(id: "next", text: "大学")
    predictionQuery.completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
      result: InputiaPersonalResult(ordered_ids: [], predictions: [prediction], context_id: target.target_id), code: nil))
    precondition(predicted != nil)
    var admission: InputiaPersonalization.Admission?
    subject.admit(prediction, from: predicted!) { admission = $0 }
    let admittedRequest = InputiaVoiceInputLauncher.pending.removeFirst()
    precondition(admittedRequest.command.kind == "admit" && admittedRequest.command.prediction_id == "next")
    admittedRequest.completion(InputiaPersonalReply(server_instance: "service", enabled: false, epoch: 2,
      result: nil, code: "policy_changed"))
    precondition(admission == nil && subject.view == nil && !subject.allowed)
    subject.stop()
    let fresh = InputiaPersonalization()
    fresh.start()
    InputiaVoiceInputLauncher.pending.removeFirst().completion(reply())
    var freshView: InputiaPersonalization.View?
    fresh.query(target: target, code: "", candidates: []) { freshView = $0 }
    drain()
    InputiaVoiceInputLauncher.pending.removeFirst().completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
      result: InputiaPersonalResult(ordered_ids: [], predictions: [prediction], context_id: target.target_id), code: nil))
    var validAdmission: InputiaPersonalization.Admission?
    fresh.admit(prediction, from: freshView!) { validAdmission = $0 }
    InputiaVoiceInputLauncher.pending.removeFirst().completion(InputiaPersonalReply(server_instance: "service", enabled: true, epoch: 1,
      result: InputiaPersonalResult(ordered_ids: nil, predictions: nil, context_id: target.target_id,
        admitted: true, prediction_id: prediction.id), code: nil))
    precondition(validAdmission != nil && fresh.admissionIsCurrent(validAdmission!))
    fresh.bind(InputiaVoiceTarget(target_id: "other-field"))
    precondition(!fresh.admissionIsCurrent(validAdmission!))
    fresh.stop()
    print("personalizationFlowSelfCheck=true coalescesLatest=true staleDiscard=true targetReset=true admitRevocation=true nativeInput=false")
  }
}
