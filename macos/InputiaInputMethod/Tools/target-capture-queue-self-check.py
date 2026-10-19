#!/usr/bin/env python3
"""无 GUI：运行实际捕获排队/传输方法，仅替换连接与权限证明，验证撤销和清理。"""

import argparse
from pathlib import Path
import re
import subprocess
import tempfile


root = Path(__file__).resolve().parents[1]
repo = root.parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--launcher-source", type=Path)
args = parser.parse_args()
source = (args.launcher_source or root / "Sources/InputiaInputMethod/InputiaVoiceInputLauncher.swift").read_text()


def declaration(text: str, marker: str) -> str:
    start = text.index(marker)
    brace = text.index("{", start)
    depth, end = 1, brace + 1
    while depth:
        depth += (text[end] == "{") - (text[end] == "}")
        end += 1
    return text[start:end].replace("fileprivate ", "").replace("private ", "")


has_queue = "final class InputiaTargetCaptureQueue" in source
if has_queue:
    queue_source = "\n".join(declaration(source, marker) for marker in [
        "final class InputiaTargetCaptureScope", "final class InputiaTargetCaptureTicket",
        "final class InputiaTargetCaptureQueue",
    ])
else:
    # 修前对照只提供调用端代数；旧targetBridge本身仍完整执行，未替换其排队行为。
    queue_source = """
final class InputiaTargetCaptureTicket {}
final class InputiaTargetCaptureScope {
  func invalidate() {}
  func ticket() -> InputiaTargetCaptureTicket { .init() }
}
"""
diagnostics = declaration(source, "enum InputiaPersonalizationDiagnostics")
bridge = declaration(source, "  static func targetBridge(")
broker_source = (repo / "src-tauri/src/ime_target_broker.rs").read_text()
reason = declaration(broker_source, "fn reason(")
reason_codes = re.findall(r'=> "(target_[a-z_]+)"', reason)

fixture = r'''
import Foundation
import OSLog
enum InputiaVoiceServiceError: Error { case profile, handshake, policy }
enum InputiaConnectionError: Error { case mainThread, endpoint, io, timeout, invalidFrame }
__DIAGNOSTICS__
__QUEUE__
struct Target { let target_id: String }
struct InputiaTargetBridgeCommand { let kind: String; var target_id: String? = nil }
struct InputiaTargetBridgeReply {
  var ready = true, server_instance = "synthetic-service"
  var permission_epoch: UInt64 = 1
  var deadline = ProcessInfo.processInfo.systemUptime + 10
  var code: String?
  var target: Target? = Target(target_id: "synthetic-opaque")
}
final class InputiaPermissionLifecycle {
  static let shared = InputiaPermissionLifecycle()
  private let lock = NSLock()
  private var value: UInt64 = 1
  var epoch: UInt64 {
    get { lock.lock(); defer { lock.unlock() }; return value }
    set { lock.lock(); value = newValue; lock.unlock() }
  }
  var allowsServiceConnection: Bool { true }
  func permits(_ epoch: UInt64) -> Bool { self.epoch == epoch }
  func matchesService(server: String, epoch: UInt64) -> Bool { server == "synthetic-service" && self.epoch == epoch }
}
enum InputiaPersonalMainDelivery {
  static func deliver(_ body: @escaping () -> Void) { DispatchQueue.main.async(execute: body) }
}
final class Statistics {
  static let shared = Statistics()
  private let lock = NSLock()
  private var counts: [String: Int] = [:]
  func record(_ key: String) { lock.lock(); counts[key, default: 0] += 1; lock.unlock() }
  func count(_ key: String) -> Int { lock.lock(); defer { lock.unlock() }; return counts[key, default: 0] }
  func reset() { lock.lock(); counts.removeAll(); lock.unlock() }
}
final class InputiaVoiceServiceConnection {
  static var onCapture: (() -> InputiaTargetBridgeReply)?
  func targetBridge(_ command: InputiaTargetBridgeCommand) throws -> InputiaTargetBridgeReply {
    Statistics.shared.record(command.kind)
    if command.kind == "capture", let action = Self.onCapture { return action() }
    return InputiaTargetBridgeReply()
  }
  func close() { Statistics.shared.record("close") }
}
enum InputiaVoiceInputLauncher {
  static let targetQueue = DispatchQueue(label: "synthetic-capture-transport")
  __CAPTURE_QUEUE_FIELD__
  static var targetConnection: InputiaVoiceServiceConnection?
  static var onOpen: (() -> Void)?
  static func openAuthenticatedConnection() throws -> InputiaVoiceServiceConnection {
    Statistics.shared.record("open"); onOpen?(); return InputiaVoiceServiceConnection()
  }
  static func releaseTarget(_ id: String) { Statistics.shared.record("release-delivery") }
  __BRIDGE__
}
func send(_ ticket: InputiaTargetCaptureTicket, completion: @escaping (InputiaTargetBridgeReply?) -> Void) {
  InputiaVoiceInputLauncher.targetBridge(.init(kind: "capture"), __TICKET_ARGUMENT__ completion: completion)
}
func pumpUntil(_ done: () -> Bool) {
  let limit = ProcessInfo.processInfo.systemUptime + 3
  while !done(), ProcessInfo.processInfo.systemUptime < limit { RunLoop.main.run(until: Date().addingTimeInterval(0.002)) }
  precondition(done(), "合成排队工作必须有界完成")
}
func reset() {
  InputiaVoiceInputLauncher.targetQueue.sync {}
  InputiaVoiceInputLauncher.targetConnection = nil
  InputiaVoiceInputLauncher.onOpen = nil
  InputiaVoiceServiceConnection.onCapture = nil
  InputiaPermissionLifecycle.shared.epoch = 1
  Statistics.shared.reset()
}
func holdQueue() -> DispatchSemaphore {
  let entered = DispatchSemaphore(value: 0), release = DispatchSemaphore(value: 0)
  InputiaVoiceInputLauncher.targetQueue.async { entered.signal(); release.wait() }
  precondition(entered.wait(timeout: .now() + 1) == .success)
  return release
}

reset()
let cancelledScope = InputiaTargetCaptureScope(), held = holdQueue()
var cancelledCallbacks = 0, cancelledResults = 0
for _ in 0..<8 { send(cancelledScope.ticket()) { result in if result == nil { cancelledResults += 1 }; cancelledCallbacks += 1 } }
cancelledScope.invalidate()
held.signal()
pumpUntil { cancelledCallbacks == 8 }
precondition(Statistics.shared.count("capture") == 0 && Statistics.shared.count("open") == 0,
  "尚未执行且已过时的捕获不能再打开连接或发送IPC/AX")
precondition(cancelledResults == 8, "每个已取消请求必须准确回调一次nil")
print("cancelledQueuedCaptureHasNoConnectionOrIPC=true")

#if HAS_CAPTURE_QUEUE
reset()
let currentScope = InputiaTargetCaptureScope(), composingHold = holdQueue()
var staleCallbacks = 0, currentCallbacks = 0
for _ in 0..<40 {
  send(currentScope.ticket()) { result in precondition(result == nil); staleCallbacks += 1 }
  currentScope.invalidate()
}
send(currentScope.ticket()) { result in precondition(result?.ready == true); currentCallbacks += 1 }
composingHold.signal()
pumpUntil { staleCallbacks == 40 && currentCallbacks == 1 }
precondition(Statistics.shared.count("capture") == 1 && Statistics.shared.count("open") == 1,
  "快打遗留捕获应被剔除，仅当前请求执行完整传输证明")
print("obsoleteCompositionBacklogCoalesced=true currentCaptureStillProven=true callbacksOnce=true")

reset()
let capacityScope = InputiaTargetCaptureScope(), capacityHold = holdQueue()
var accepted = 0, rejected = 0, capturesAtControl = -1
for _ in 0..<20 { send(capacityScope.ticket()) { reply in if reply == nil { rejected += 1 } else { accepted += 1 } } }
InputiaVoiceInputLauncher.targetQueue.async {
  let count = Statistics.shared.count("capture")
  DispatchQueue.main.async { capturesAtControl = count }
}
capacityHold.signal()
pumpUntil { accepted + rejected == 20 && capturesAtControl >= 0 }
precondition(accepted == 16 && rejected == 4 && Statistics.shared.count("capture") == 16,
  "最多16个待执行捕获，超限明确拒绝且不得生成重试任务")
precondition(capturesAtControl <= 1, "排空捕获必须让出原串行队列，不阻塞释放/核验")
print("boundedCaptureCapacity=true controlQueueRemainsFair=true")

reset()
let runningScope = InputiaTargetCaptureScope(), running = DispatchSemaphore(value: 0), finish = DispatchSemaphore(value: 0)
InputiaVoiceServiceConnection.onCapture = { running.signal(); finish.wait(); return .init() }
var runningCallbacks = 0
send(runningScope.ticket()) { result in precondition(result?.ready == true); runningCallbacks += 1 }
precondition(running.wait(timeout: .now() + 1) == .success)
runningScope.invalidate(); finish.signal()
pumpUntil { runningCallbacks == 1 }
precondition(Statistics.shared.count("capture") == 1, "已经发送的捕获保持原回包路径，不重试或吞掉清理机会")
print("startedCaptureCompletesWithoutReplay=true")

reset()
let handshakeScope = InputiaTargetCaptureScope()
InputiaVoiceInputLauncher.onOpen = { handshakeScope.invalidate() }
var handshakeCallbacks = 0
send(handshakeScope.ticket()) { result in precondition(result == nil); handshakeCallbacks += 1 }
pumpUntil { handshakeCallbacks == 1 }
precondition(Statistics.shared.count("open") == 1 && Statistics.shared.count("capture") == 0,
  "握手期间已取消且尚未发出的capture也不得进入AX")
print("cancelledDuringHandshakeSkipsCapture=true")

reset()
InputiaVoiceServiceConnection.onCapture = { .init(deadline: ProcessInfo.processInfo.systemUptime - 1) }
var expiredCallbacks = 0
send(InputiaTargetCaptureScope().ticket()) { result in precondition(result == nil); expiredCallbacks += 1 }
pumpUntil { expiredCallbacks == 1 }
precondition(Statistics.shared.count("capture") == 1 && Statistics.shared.count("release") == 1,
  "捕获返回过期目标仍按原路径释放，不能因新队列而泄漏")

reset()
var revokedCallbacks = 0
send(InputiaTargetCaptureScope().ticket()) { result in precondition(result == nil); revokedCallbacks += 1 }
InputiaVoiceInputLauncher.targetQueue.sync {}
InputiaPermissionLifecycle.shared.epoch = 2
pumpUntil { revokedCallbacks == 1 }
precondition(Statistics.shared.count("capture") == 1 && Statistics.shared.count("release-delivery") == 1,
  "回主线程前权限变化必须释放已经返回的目标，不可派发旧证明")
print("expiredAndRevokedCaptureTargetsReleased=true")

reset()
let duplicateTicket = InputiaTargetCaptureScope().ticket(), duplicateHold = holdQueue()
var duplicateSuccess = 0, duplicateCancelled = 0
for _ in 0..<2 { send(duplicateTicket) { result in if result == nil { duplicateCancelled += 1 } else { duplicateSuccess += 1 } } }
duplicateHold.signal()
pumpUntil { duplicateSuccess + duplicateCancelled == 2 }
precondition(duplicateSuccess == 1 && duplicateCancelled == 1 && Statistics.shared.count("capture") == 1)
let trace1 = InputiaPersonalizationDiagnostics.targetTrace(operation: "arbitrary-sensitive-input")
let trace2 = InputiaPersonalizationDiagnostics.targetTrace(operation: "capture")
precondition(trace1.operation == "other" && trace2.id > trace1.id)
precondition(InputiaPersonalizationDiagnostics.milliseconds(.nan) == 0
  && InputiaPersonalizationDiagnostics.milliseconds(.infinity) == 0
  && InputiaPersonalizationDiagnostics.milliseconds(-1) == 0
  && InputiaPersonalizationDiagnostics.milliseconds(999999) == 3_600_000)
for code in __RUST_REASON_CODES__ { precondition(InputiaPersonalizationDiagnostics.reasons.contains(code), "真实Rust拒绝码必须保留于固定白名单") }
print("duplicateTicketCannotReplay=true traceContainsOnlyFixedMetadata=true rustReasonsRemainVisible=true")

reset()
let invalidScope = InputiaTargetCaptureScope(), invalidTicket = invalidScope.ticket()
invalidScope.invalidate()
var noncaptureCallbacks = 0
for kind in ["validate", "release", "status"] {
  InputiaVoiceInputLauncher.targetBridge(.init(kind: kind, target_id: "synthetic-existing-target"),
    captureTicket: invalidTicket) { result in precondition(result?.ready == true); noncaptureCallbacks += 1 }
}
pumpUntil { noncaptureCallbacks == 3 }
precondition(Statistics.shared.count("validate") == 1 && Statistics.shared.count("release") == 1
  && Statistics.shared.count("status") == 1 && Statistics.shared.count("capture") == 0
  && Statistics.shared.count("release-delivery") == 0)
print("invalidCaptureTicketDoesNotCancelNoncapture=true noOtherTargetCleanup=true")

reset()
let parallelHold = holdQueue(), producers = DispatchGroup(), parallelScope = InputiaTargetCaptureScope()
var parallelCallbacks = 0, parallelAccepted = 0, parallelRejected = 0
for index in 0..<64 {
  producers.enter()
  DispatchQueue.global().async {
    send(parallelScope.ticket()) { result in
      Statistics.shared.record("callback-\(index)")
      parallelCallbacks += 1
      if result == nil { parallelRejected += 1 } else { parallelAccepted += 1 }
    }
    producers.leave()
  }
}
precondition(producers.wait(timeout: .now() + 2) == .success)
parallelHold.signal()
pumpUntil { parallelCallbacks == 64 }
precondition(parallelAccepted == 16 && parallelRejected == 48 && Statistics.shared.count("capture") == 16)
precondition((0..<64).allSatisfy { Statistics.shared.count("callback-\($0)") == 1 })
print("parallelProducersRespectCapacity=true everyRequestCompletesOnce=true")
#endif
'''

with tempfile.TemporaryDirectory(prefix="inputia-target-capture-queue-") as temporary:
    directory = Path(temporary)
    swift = directory / "main.swift"
    swift.write_text(fixture.replace("__QUEUE__", queue_source).replace("__DIAGNOSTICS__", diagnostics)
        .replace("__BRIDGE__", bridge)
        .replace("__CAPTURE_QUEUE_FIELD__", "static let targetCaptures = InputiaTargetCaptureQueue()" if has_queue else "")
        .replace("__TICKET_ARGUMENT__", "captureTicket: ticket," if has_queue else "")
        .replace("__RUST_REASON_CODES__", str(reason_codes).replace("'", '"')))
    binary = directory / "check"
    flags = ["-D", "HAS_CAPTURE_QUEUE"] if has_queue else []
    subprocess.run(["/usr/bin/swiftc", "-D", "INPUTIA_PAIRED_BUILD", *flags, str(swift), "-o", str(binary)], check=True)
    result = subprocess.run([str(binary)], check=False)
    if result.returncode:
        raise SystemExit(result.returncode)
    if has_queue:
        enum_source = declaration((repo / "src-tauri/src/unified_target.rs").read_text(), "pub enum PendingReason")
        test_source = declaration(broker_source, "    fn native_target_rejections_have_stable_diagnostic_codes()")
        rust = directory / "mapping.rs"
        rust.write_text("#[derive(Debug, Clone, Copy)]\n" + enum_source + "\n" + reason + "\n#[test]\n" + test_source)
        rust_binary = directory / "mapping"
        subprocess.run(["rustc", "--test", str(rust), "-o", str(rust_binary)], check=True)
        subprocess.run([str(rust_binary)], check=True)
