#!/usr/bin/env python3
"""真实监听 timer→注册→握手屏障→Personalization；只替换原生目标和传输边界。"""

import argparse
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
sources = root / "Sources/InputiaInputMethod"
parser = argparse.ArgumentParser()
parser.add_argument("--source-directory", type=Path, default=sources)
parser.add_argument("--scenario", choices=["gap", "rejected", "replacement", "active", "protocol"])
args = parser.parse_args()
launcher = (args.source_directory / "InputiaVoiceInputLauncher.swift").read_text()
connection = (args.source_directory / "InputiaVoiceServiceConnection.swift").read_text()
memory = (sources / "InputiaManagedMemory.swift").read_text()
host = (sources / "main.swift").read_text()


def declaration(source: str, marker: str) -> str:
    start = source.index(marker)
    brace = source.index("{", start)
    depth, end = 1, brace + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end].replace("private ", "")


types = "\n".join(declaration(connection, marker) for marker in [
    "struct InputiaVoiceTermsVersion", "struct InputiaVoicePolicyBarrier", "private struct InputiaVoicePolicyAck",
    "protocol InputiaSharedStateBarrierApplying", "enum InputiaVoiceServiceError",
    "enum InputiaVoiceShortcutActivation", "struct InputiaHostShortcutEdge", "struct InputiaHostShortcutLease",
    "struct InputiaHostShortcutTrigger", "private struct InputiaHostShortcutCommand",
    "private struct InputiaHostShortcutRequest", "private struct InputiaHostShortcutReply",
])
methods = "\n".join(declaration(connection, marker) for marker in [
    "  func synchronizePolicy(", "  private static func applyAndAcknowledge(",
    "  func registerShortcutLease(", "  func retireShortcutLease(", "  func pollShortcut(",
    "  func rejectUnconsumedShortcut(", "  private func shortcutRequest(", "  func close()",
])
scope = declaration(connection, "  private struct OwnedContextScope")
scope += "\n  static var appliedOwnedContextScope: OwnedContextScope?"
listen = declaration(launcher, "  static func startShortcutListening(")
host_methods = "\n".join(declaration(host, marker) for marker in [
    "  private func discardPreparedVoiceTarget()", "  func rejectShortcutRegistrationTarget(",
])
model_fixture = (root / "Tools/InputiaPersonalizationFlowSelfCheck.swift").read_text()
model_types = model_fixture[model_fixture.index("struct InputiaPersonalCandidate"):model_fixture.index("enum InputiaVoiceInputLauncher")]

fixture = r'''
import Foundation
import OSLog
import Darwin
struct InputiaVoiceTarget: Codable, Equatable {
  let target_id: String
  var host_instance = "synthetic-host"
  var field_id: String? = "synthetic-field"
}
__TYPES__
__MODEL_TYPES__
enum InputiaPersonalizationDiagnostics {
  static func record(_ phase: String, _ reason: String, flags: Int = 0, count: Int = 0) {}
}
enum InputiaMemoryError: Error { case unavailable }
struct InputiaMemoryPolicy { let server_instance: String; let profile_id: String; let policy_epoch: UInt64 }
__MEMORY_BARRIER__
final class InputiaPermissionLifecycle {
  static let shared = InputiaPermissionLifecycle()
  var epoch: UInt64 = 1
  func permits(_ epoch: UInt64) -> Bool { self.epoch == epoch }
  func matchesService(server: String, epoch: UInt64) -> Bool { permits(epoch) }
}
final class InputiaSharedTermsMemory {
  static let shared = InputiaSharedTermsMemory()
  private let lock = NSLock()
  private var generation: UInt64 = 0
  func clear() { lock.lock(); generation += 1; lock.unlock() }
  func ticket() -> UInt64 { lock.lock(); defer { lock.unlock() }; return generation }
}
struct InputiaSharedTermsSnapshot {}
struct SyntheticTargetCommand { let kind: String; let target: InputiaVoiceTarget; let purpose: String }
struct SyntheticTargetReply { let ready = false, server_instance = "service"; let permission_epoch: UInt64 = 1; let deadline: TimeInterval = 0 }
struct SyntheticDelivery {}
struct SyntheticHello { let instance_id = "service", profile_id = "synthetic-profile"; let policy_epoch: UInt64 = 1; let capabilities = [String]() }
final class SyntheticState: InputiaSharedStateBarrierApplying {
  func applySharedStateBarrier(_ barrier: InputiaVoicePolicyBarrier) throws {}
}
final class NativeService {
  static let shared = NativeService()
  private let lock = NSLock()
  private var edited = Set<String>(), mode: String?
  private var counts: [String: Int] = [:]
  private var retireIDs: [String] = []
  private var registered: InputiaHostShortcutLease?
  private var pendingStop = false
  func record(_ key: String) { lock.lock(); counts[key, default: 0] += 1; lock.unlock() }
  func count(_ key: String) -> Int { lock.lock(); defer { lock.unlock() }; return counts[key, default: 0] }
  func reject(_ id: String) { lock.lock(); edited.insert(id); lock.unlock() }
  func corrupt(_ next: String) { lock.lock(); mode = next; lock.unlock() }
  func stop() { lock.lock(); pendingStop = true; lock.unlock() }
  func retired() -> [String] { lock.lock(); defer { lock.unlock() }; return retireIDs }
  func respond(_ request: InputiaHostShortcutRequest) throws -> Data {
    lock.lock(); defer { lock.unlock() }
    let command = request.shortcut
    var reply: [String: Any] = ["request_id": request.request_id]
    let fault = mode; mode = nil
    if fault == "transport" { throw InputiaVoiceServiceError.handshake }
    if command.kind == "register", let lease = command.lease {
      counts["register:" + lease.target.target_id, default: 0] += 1
      if edited.contains(lease.target.target_id) || fault != nil {
        reply["status"] = "rejected"; reply["code"] = "unauthorized"
      } else {
        registered = lease
        reply["status"] = "registered"; reply["lease_id"] = lease.lease_id; reply["lease_epoch"] = lease.lease_epoch
      }
    } else if command.kind == "retire" {
      if let current = registered, command.lease_id == current.lease_id && command.lease_epoch == current.lease_epoch {
        retireIDs.append(current.target.target_id); registered = nil
      }
      reply["status"] = "retired"
    } else if command.kind == "poll" {
      counts["poll", default: 0] += 1
      if pendingStop, let lease = registered {
        pendingStop = false
        reply["status"] = "trigger"
        reply["trigger"] = ["trigger_id": "synthetic-stop", "session_id": "synthetic-session", "starts_session": false,
          "lease_id": lease.lease_id, "lease_epoch": lease.lease_epoch,
          "target": ["target_id": lease.target.target_id, "host_instance": "synthetic-host", "field_id": "synthetic-field"],
          "binding_id": "transcribe", "hotkey_string": "ctrl+shift+space", "is_pressed": true,
          "activation": "toggle", "pressed_at_unix_ms": 1, "hold_threshold_ms": 300,
          "server_instance": "service", "client_instance": "synthetic-host", "policy_epoch": 1]
      } else { reply["status"] = "empty" }
    } else { reply["status"] = "retired" }
    if fault == "wrong-request" { reply["request_id"] = "other-request" }
    if fault == "unknown-code" { reply["code"] = "unknown" }
    if fault == "extra-lease" { reply["lease_id"] = "unexpected" }
    if fault == "bad-frame" { return Data("not-json".utf8) }
    return try JSONSerialization.data(withJSONObject: reply)
  }
}
final class SyntheticTransport {
  private var request: InputiaHostShortcutRequest?
  private var closed = false
  func write<T>(_ value: T) throws {
    if let command = value as? InputiaHostShortcutRequest { request = command }
    if value is InputiaVoicePolicyAck { NativeService.shared.record("ack") }
  }
  func read<T: Decodable>(_ type: T.Type) throws -> T {
    if T.self == InputiaVoicePolicyBarrier.self {
      return InputiaVoicePolicyBarrier(barrier_id: String(repeating: "a", count: 64),
        version: .init(policy_epoch: 1, learning_generation: 1), clear_shared_personalization: true) as! T
    }
    guard let request else { throw InputiaVoiceServiceError.handshake }
    return try JSONDecoder().decode(type, from: NativeService.shared.respond(request))
  }
  func close() { if !closed { closed = true; NativeService.shared.record("close") } }
}
final class InputiaVoiceServiceConnection {
  static let processInstance = "synthetic-host"
  let connection = SyntheticTransport(), server = SyntheticHello()
  var privacyState: InputiaSharedStateBarrierApplying?
  var locallyAppliedVersion: InputiaVoiceTermsVersion?
  __SCOPE__
  __METHODS__
  static func sharedTermsConnectionMatches(server: String, primaryServer: String, version: InputiaVoiceTermsVersion?, primaryVersion: InputiaVoiceTermsVersion?) -> Bool { false }
  func targetBridge(_ command: SyntheticTargetCommand) throws -> SyntheticTargetReply { .init() }
  func fetchSharedTerms(lease: InputiaHostShortcutLease, leaseDeadline: TimeInterval) throws -> InputiaSharedTermsSnapshot? { nil }
}
enum InputiaVoiceInputLauncher {
  static let shortcutDiagnostic = Logger(subsystem: "synthetic.shortcut", category: "test")
  static let shortcutQueue = DispatchQueue(label: "synthetic.shortcut")
  static let voiceQueue = DispatchQueue(label: "synthetic.voice"), sharedTermsQueue = DispatchQueue(label: "synthetic.terms")
  static var shortcutConnection: InputiaVoiceServiceConnection?, sharedTermsConnection: InputiaVoiceServiceConnection?
  static var shortcutLease: InputiaHostShortcutLease?, shortcutRejectedTargetID: String?
  static var shortcutLeaseEpoch: UInt64 = 0
  static var shortcutTimer: DispatchSourceTimer?
  static var shortcutCycleBusy = false, shortcutHasActiveOwner = false, sharedTermsBusy = false
  static var shortcutProviderHadTarget: Bool?
  static var shortcutRetryAfter: TimeInterval = 0, sharedTermsRequestedAt: TimeInterval = 0
  static var shortcutSession: String?, shortcutServer: String?
  static func openAuthenticatedConnection() throws -> InputiaVoiceServiceConnection {
    NativeService.shared.record("open")
    let connection = InputiaVoiceServiceConnection()
    try connection.synchronizePolicy(using: SyntheticState())
    return connection
  }
  static func clearShortcutOwnership(sessionID: String) {}
  static func releaseTarget(_ id: String) { NativeService.shared.record("release:" + id) }
  static func sendUnifiedShortcutTrigger(_ trigger: InputiaHostShortcutTrigger,
    deliver: @escaping (SyntheticDelivery, @escaping (String) -> Void) -> Void,
    status: @escaping (String) -> Void, completion: @escaping (Bool) -> Void) {
    precondition(!trigger.starts_session, "合成停止必须经过真实poll与既有session分支")
    NativeService.shared.record("stop-delivered"); completion(true)
  }
  static func personalization(_ command: InputiaPersonalCommand, deadline: TimeInterval,
    expectedServer: String? = nil, completion: @escaping (InputiaPersonalReply?) -> Void) {
    DispatchQueue.main.async {
      completion(.init(server_instance: "service", enabled: true, epoch: 1,
        result: command.kind == "query" ? .init(ordered_ids: [], predictions: [], context_id: command.context_id) : nil, code: nil))
    }
  }
  __LISTEN__
}
final class SyntheticSnapshot {
  let inputiaTarget: InputiaVoiceTarget
  var targetID: String { inputiaTarget.target_id }
  init(_ target: InputiaVoiceTarget) { inputiaTarget = target }
}
final class Controller {
  var shortcutPreparedSnapshot: SyntheticSnapshot?
  var voiceTargetSnapshots: [String: SyntheticSnapshot] = [:]
  var typedEventOrigin: SyntheticSnapshot?, typedCompositionOrigin: SyntheticSnapshot?
  var typedRetainedOrigins: [String: TimeInterval] = [:]
  var sharedTargetReady = true
  var sharedPreparedClientIdentity: ObjectIdentifier?
  var prepared: InputiaVoiceTarget? {
    get { shortcutPreparedSnapshot?.inputiaTarget }
    set { shortcutPreparedSnapshot = newValue.map(SyntheticSnapshot.init) }
  }
  __HOST_METHODS__
}
func pump(_ seconds: Double) { RunLoop.main.run(until: Date().addingTimeInterval(seconds)) }
func until(_ predicate: () -> Bool, timeout: Double = 3) {
  let end = ProcessInfo.processInfo.systemUptime + timeout
  while !predicate() && ProcessInfo.processInfo.systemUptime < end { pump(0.005) }
  precondition(predicate(), "真实timer链必须在原期限内完成")
}
@main struct ShortcutRegistrationLifecycleSelfCheck {
  static func main() throws {
    let scenario = CommandLine.arguments[1]
    let model = InputiaPersonalization()
    var barriers = 0, disabled = 0, rejected: [String] = []
    let controller = Controller(); controller.prepared = .init(target_id: "A")
    // 实际main回调只能移除匹配的prepared归属，不能撤销其他consumer持有的目标。
    let ownership = Controller(); ownership.prepared = .init(target_id: "new")
    ownership.rejectShortcutRegistrationTarget("old")
    precondition(ownership.prepared?.target_id == "new" && NativeService.shared.count("release:new") == 0)
    for owner in ["event", "composition", "retained", "voice"] {
      let value = Controller(), id = "owned-" + owner
      value.prepared = .init(target_id: id)
      switch owner {
      case "event": value.typedEventOrigin = value.shortcutPreparedSnapshot
      case "composition": value.typedCompositionOrigin = value.shortcutPreparedSnapshot
      case "retained": value.typedRetainedOrigins[id] = ProcessInfo.processInfo.systemUptime + 3
      default: value.voiceTargetSnapshots[id] = value.shortcutPreparedSnapshot
      }
      value.rejectShortcutRegistrationTarget(id)
      precondition(value.prepared == nil && !value.sharedTargetReady && NativeService.shared.count("release:" + id) == 0)
    }
    InputiaMemoryBarrier.clear = { _ in precondition(Thread.isMainThread); barriers += 1 }
    model.policyChanged = { if !model.allowed { disabled += 1 } }
    model.start(); until { model.allowed }
    InputiaVoiceInputLauncher.startShortcutListening(targetProvider: { controller.prepared },
      __REJECTION_CALLBACK__
      sharedTermsReceiver: { _, _ in }, acceptStart: { _, _ in fatalError("本测试不派发录音开始") })
    defer { InputiaVoiceInputLauncher.shortcutQueue.sync { InputiaVoiceInputLauncher.shortcutTimer?.cancel() }; model.stop() }
    until { NativeService.shared.count("register:A") > 0 }
    until { model.allowed }
    let initialBarriers = barriers, initialDisabled = disabled
    var rendered = false
    model.query(target: .init(target_id: "view"), code: "", candidates: []) { _ in rendered = true }
    until { rendered }
    let viewGeneration = model.view?.generation
    if scenario == "gap" {
      controller.prepared = nil
      pump(0.45)
      precondition(NativeService.shared.retired() == ["A"], "目标暂缺应退役已注册租约而非关闭socket")
      controller.prepared = .init(target_id: "B")
      until { NativeService.shared.count("register:B") > 0 }
    } else if scenario == "rejected" {
      NativeService.shared.reject("A")
      pump(2.65)
      print("rejectionCycleObserved opens=\(NativeService.shared.count("open")) barriers=\(barriers) disabled=\(disabled) registrations=\(NativeService.shared.count("register:A"))")
      fflush(stdout)
      precondition(NativeService.shared.count("open") == 1 && barriers == initialBarriers && disabled == initialDisabled,
        "相同失效原生目标不得触发每秒重连→握手屏障→Personalization失效循环")
      precondition(rejected == ["A"] && controller.prepared == nil, "明确拒绝必须只通知旧目标一次，停止重连注册同一目标")
      precondition(NativeService.shared.retired() == ["A"])
      controller.prepared = .init(target_id: "B")
      until { NativeService.shared.count("register:B") > 0 }
    } else if scenario == "replacement" {
      NativeService.shared.reject("B"); controller.prepared = .init(target_id: "B")
      until { rejected == ["B"] }
      until { NativeService.shared.retired() == ["A"] }
      precondition(!NativeService.shared.retired().contains("B"), "待注册失败的candidate不能冒充已注册lease退役")
    } else if scenario == "active" {
      InputiaVoiceInputLauncher.shortcutQueue.sync { InputiaVoiceInputLauncher.shortcutHasActiveOwner = true }
      NativeService.shared.reject("A")
      until { rejected == ["A"] }
      NativeService.shared.stop()
      until { NativeService.shared.count("stop-delivered") == 1 }
      precondition(NativeService.shared.retired().isEmpty, "退役协议会清active：已有录音期间不得发送")
      precondition(controller.prepared == nil)
    } else if scenario == "protocol" {
      InputiaVoiceInputLauncher.shortcutQueue.sync { InputiaVoiceInputLauncher.shortcutTimer?.cancel() }
      for fault in ["wrong-request", "unknown-code", "extra-lease", "bad-frame", "transport"] {
        var done = false, threw = false
        let beforeClose = NativeService.shared.count("close")
        DispatchQueue.global().async {
          do {
            let connection = InputiaVoiceServiceConnection()
            connection.locallyAppliedVersion = .init(policy_epoch: 1, learning_generation: 1)
            NativeService.shared.corrupt(fault)
            _ = try connection.registerShortcutLease(.init(lease_id: "fault", lease_epoch: 3,
              target: .init(target_id: "C"), issued_at_unix_ms: 1, expires_at_unix_ms: 2))
          } catch { DispatchQueue.main.async { threw = true } }
          DispatchQueue.main.async { done = true }
        }
        until { done }
        precondition(threw && NativeService.shared.count("close") == beforeClose + 1,
          "未知错误/错requestID/额外payload/坏帧/传输错误必须继续fail-closed")
      }
      print("shortcutProtocolErrorsStillClose=true noAutomaticRequestReplay=true")
      return
    }
    precondition(NativeService.shared.count("open") == 1 && NativeService.shared.count("close") == 0,
      "目标生命周期不能制造认证连接重建循环")
    precondition(barriers == initialBarriers && disabled == initialDisabled && model.allowed,
      "目标暂缺或已确认拒绝不得再次握手→全局屏障→allowed=false")
    precondition(model.view?.generation == viewGeneration, "不得清空其他个性化consumer的view")
    InputiaPermissionLifecycle.shared.epoch += 1
    let registrations = NativeService.shared.count("register:B")
    controller.prepared = .init(target_id: "B"); pump(0.25)
    precondition(NativeService.shared.count("register:B") == registrations, "旧监听不能跨权限epoch继续工作")
    print("shortcutLifecycle=\(scenario) realTimerAndWireAndBarrierAndPersonalization=true opens=1 noRepeatedPrivacyClear=true")
  }
}
'''

callback = """targetRejected: { id in
        precondition(Thread.isMainThread)
        rejected.append(id)
        controller.rejectShortcutRegistrationTarget(id)
      },""" if "targetRejected:" in listen else ""
for marker, value in {
    "__TYPES__": types, "__MODEL_TYPES__": model_types,
    "__MEMORY_BARRIER__": declaration(memory, "enum InputiaMemoryBarrier"),
    "__SCOPE__": scope, "__METHODS__": methods, "__LISTEN__": listen,
    "__HOST_METHODS__": host_methods,
    "__REJECTION_CALLBACK__": callback,
}.items():
    fixture = fixture.replace(marker, value)

with tempfile.TemporaryDirectory(prefix="inputia-shortcut-registration-") as directory:
    path = Path(directory)
    (path / "main.swift").write_text(fixture)
    binary = path / "check"
    subprocess.run(["swiftc", "-D", "INPUTIA_PAIRED_BUILD", "-parse-as-library",
        str(sources / "InputiaPersonalization.swift"), str(path / "main.swift"), "-o", str(binary)], check=True)
    for scenario in [args.scenario] if args.scenario else ["gap", "rejected", "replacement", "active", "protocol"]:
        subprocess.run([str(binary), scenario], check=True)
print("shortcutRegistrationProductionLifecyclePassed=true noGUIOrUserProfile=true")
