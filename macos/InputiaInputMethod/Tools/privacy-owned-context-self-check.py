#!/usr/bin/env python3
"""无 GUI：执行真实握手与屏障方法，仅替换传输、存储和宿主清理边界。"""

import argparse
from pathlib import Path
import re
import subprocess
import tempfile


root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument("--connection-source", type=Path)
args = parser.parse_args()
source = (args.connection_source or root / "Sources/InputiaInputMethod/InputiaVoiceServiceConnection.swift").read_text()
memory = (root / "Sources/InputiaInputMethod/InputiaManagedMemory.swift").read_text()


def declaration(text: str, marker: str) -> str:
    start = text.index(marker)
    brace = text.index("{", start)
    depth, end = 1, brace + 1
    while depth:
        depth += (text[end] == "{") - (text[end] == "}")
        end += 1
    return text[start:end].replace("private ", "")


has_scope = "private struct OwnedContextScope" in source
scope = ""
if has_scope:
    scope = declaration(source, "  private struct OwnedContextScope")
    scope += "\n" + re.search(r"  private static var appliedOwnedContextScope[^\n]*", source)[0].replace("private ", "")
types = "\n".join(declaration(source, marker) for marker in [
    "struct InputiaVoiceTermsVersion", "struct InputiaVoicePolicyBarrier",
    "private struct InputiaVoicePolicyAck", "protocol InputiaSharedStateBarrierApplying",
    "enum InputiaVoiceServiceError",
])
apply = declaration(source, "  private static func applyAndAcknowledge(")
synchronize = declaration(source, "  func synchronizePolicy(")

fixture = r'''
import Foundation
__TYPES__
enum InputiaMemoryError: Error { case unavailable }
struct InputiaMemoryPolicy {
  let server_instance: String
  let profile_id: String
  let policy_epoch: UInt64
}
__MEMORY_BARRIER__
enum Trace {
  static var order: [String] = []
  static var actions: [String] = []
  static var notifiedOnMain = true
  static var failMemory = false
  static func reset() { order = []; actions = []; notifiedOnMain = true }
}
final class InputiaSharedTermsMemory {
  static let shared = InputiaSharedTermsMemory()
  func clear() { Trace.order.append("shared") }
}
final class SyntheticState: InputiaSharedStateBarrierApplying {
  var reject = false
  func applySharedStateBarrier(_ barrier: InputiaVoicePolicyBarrier) throws {
    Trace.order.append("storage")
    if reject { throw InputiaVoiceServiceError.policy }
  }
}
struct SyntheticHello {
  let instance_id: String
  let profile_id: String
  let policy_epoch: UInt64
}
final class SyntheticTransport {
  var barrier: InputiaVoicePolicyBarrier
  var failSend = false
  init(_ barrier: InputiaVoicePolicyBarrier) { self.barrier = barrier }
  func read<T>(_ type: T.Type) throws -> T { barrier as! T }
  func write<T>(_ value: T) throws {
    let ack = value as! InputiaVoicePolicyAck
    precondition(ack.shared_cache_cleared && ack.offline_queue_revalidated)
    precondition(ack.version == barrier.version && ack.barrier_id == barrier.barrier_id)
    precondition(Trace.order.last == "notification", "所有本地清理与通知必须先于 ACK")
    Trace.order.append("ack")
    if failSend { throw InputiaVoiceServiceError.policy }
  }
  func close() { Trace.order.append("close") }
}
final class InputiaVoiceServiceConnection {
  __SCOPE__
  let connection: SyntheticTransport
  var privacyState: InputiaSharedStateBarrierApplying?
  var locallyAppliedVersion: InputiaVoiceTermsVersion?
  let server: SyntheticHello
  init(_ barrier: InputiaVoicePolicyBarrier, server: String, profile: String) {
    connection = SyntheticTransport(barrier)
    self.server = .init(instance_id: server, profile_id: profile, policy_epoch: barrier.version.policy_epoch)
  }
  __SYNCHRONIZE__
  __APPLY__
}
func barrier(_ epoch: UInt64 = 3, _ generation: UInt64 = 7, id: String = "a") -> InputiaVoicePolicyBarrier {
  .init(barrier_id: String(repeating: id, count: 64), version: .init(policy_epoch: epoch, learning_generation: generation),
    clear_shared_personalization: true)
}
func handshake(_ value: InputiaVoicePolicyBarrier = barrier(), server: String = "synthetic-server",
  profile: String = "synthetic-profile", state: SyntheticState = .init(), failSend: Bool = false) throws {
  let client = InputiaVoiceServiceConnection(value, server: server, profile: profile)
  client.connection.failSend = failSend
  try client.synchronizePolicy(using: state)
  precondition(client.locallyAppliedVersion == value.version)
}
func realBarrier(_ value: InputiaVoicePolicyBarrier = barrier(), server: String? = "synthetic-server",
  profile: String? = "synthetic-profile", state: SyntheticState = .init(), minimum: UInt64 = 3) throws {
  // 不传 isHandshake：测试真实撤销调用的生产默认值，不能伪造为握手。
  try InputiaVoiceServiceConnection.applyAndAcknowledge(value, minimumEpoch: minimum, state: state,
    privacyServer: server, privacyProfile: profile) { ack in
      precondition(ack.shared_cache_cleared && ack.offline_queue_revalidated)
      precondition(Trace.order.last == "notification")
      Trace.order.append("ack")
    }
}
func expectAction(_ expected: String, handshake: Bool = true) {
  precondition(Trace.actions == [expected], "自有前文动作必须是 \(expected)，实际 \(Trace.actions)")
  let prefix = handshake ? ["shared", "shared"] : ["shared"]
  precondition(Trace.order == prefix + ["storage", "memory", "notification", "ack"],
    "即使 preserve，仍必须依次完成共享缓存/持久状态/宿主目标清理、通知、ACK")
  precondition(Trace.notifiedOnMain)
}
@main struct PrivacyOwnedContextSelfCheck {
  static func main() throws {
    InputiaMemoryBarrier.clear = { _ in
      precondition(Thread.isMainThread)
      Trace.order.append("memory")
      if Trace.failMemory { throw InputiaMemoryError.unavailable }
    }
    let observer = NotificationCenter.default.addObserver(forName: .init("InputiaPrivacyRevoked"), object: nil, queue: nil) {
      notification in
      Trace.notifiedOnMain = Trace.notifiedOnMain && Thread.isMainThread
      Trace.actions.append(notification.userInfo?["owned_context_action"] as? String ?? "missing")
      Trace.order.append("notification")
    }
    defer { NotificationCenter.default.removeObserver(observer) }

    let rejected = SyntheticState(); rejected.reject = true
    do { try handshake(state: rejected); fatalError("失败持久屏障不能 ACK") } catch {}
    precondition(Trace.actions.isEmpty && !Trace.order.contains("memory") && !Trace.order.contains("ack"))
    Trace.reset(); Trace.failMemory = true
    do { try handshake(); fatalError("失败宿主屏障不能 ACK") } catch {}
    precondition(Trace.actions.isEmpty && !Trace.order.contains("ack"))
    Trace.failMemory = false; Trace.reset()
    try handshake(); expectAction("clear_prefix")
    print("firstSuccessfulHandshakeClearsOnlyPriorPrefix=true failedCleanupNeverAdvancesScopeOrACK=true")

    Trace.reset(); try handshake(); expectAction("preserve")
    print("sameCompleteScopeHandshakePreservesOwnedContextOnly=true sharedAndTargetCleanupStillRequired=true")
    Trace.reset(); try realBarrier(); expectAction("clear_all", handshake: false)
    print("sameEpochRealRevocationClearsAllOwnedContext=true")
    Trace.reset(); try handshake(); expectAction("preserve")
    Trace.reset(); try handshake(barrier(3, 8)); expectAction("clear_all")
    print("learningGenerationChangeClearsAll=true")
    Trace.reset(); try handshake(barrier(4, 8)); expectAction("clear_all")
    print("policyEpochChangeClearsAll=true")
    Trace.reset(); try handshake(barrier(4, 8), server: "synthetic-server-next"); expectAction("clear_all")
    print("serverChangeClearsAll=true")
    Trace.reset(); try handshake(barrier(4, 8), server: "synthetic-server-next", profile: "synthetic-profile-next")
    expectAction("clear_all")
    print("profileChangeClearsAll=true")
    Trace.reset(); try handshake(barrier(4, 8, id: "b"), server: "synthetic-server-next", profile: "synthetic-profile-next")
    expectAction("preserve")
    print("HandshakeIdentityUsesFullScopeRatherThanBarrierID=true")

    Trace.reset(); Trace.failMemory = true
    do { try handshake(barrier(5, 9)); fatalError("失败清理不能推进进程 scope") } catch {}
    precondition(Trace.actions.isEmpty && !Trace.order.contains("ack"))
    Trace.failMemory = false; Trace.reset()
    try handshake(barrier(4, 8), server: "synthetic-server-next", profile: "synthetic-profile-next"); expectAction("preserve")
    Trace.reset()
    do { try realBarrier(server: nil); fatalError("未知 scope 不能 ACK") } catch {}
    precondition(Trace.actions.isEmpty && !Trace.order.contains("ack"))
    print("failedOrMissingScopeCannotGrantPreservation=true")

    for invalid in [barrier(2, 8), barrier(4, 8, id: "G"),
      InputiaVoicePolicyBarrier(barrier_id: String(repeating: "a", count: 64), version: barrier().version,
        clear_shared_personalization: false)] {
      Trace.reset()
      do { try realBarrier(invalid); fatalError("原屏障验证不得被绕过") } catch {}
      precondition(Trace.order.isEmpty && Trace.actions.isEmpty)
    }
    Trace.reset()
    do { try realBarrier(barrier(4, 7), state: rejected); fatalError("持久版本拒绝不能 ACK") } catch {}
    precondition(Trace.actions.isEmpty && !Trace.order.contains("ack"))
    print("existingMalformedStaleAndStorageRejectionRemainFailClosed=true")

    Trace.reset()
    do { try handshake(barrier(5, 9), failSend: true); fatalError("传输失败必须传播") } catch {}
    precondition(Trace.actions == ["clear_all"] && Trace.order.suffix(2) == ["ack", "close"])
    Trace.reset(); try handshake(barrier(5, 9)); expectAction("preserve")
    print("completedLocalCleanupIsRememberedEvenIfACKSendFails=true")

    Trace.reset()
    var completed = false, failed = false
    DispatchQueue.global().async {
      do { try handshake(barrier(5, 9)) } catch { DispatchQueue.main.async { failed = true } }
      DispatchQueue.main.async { completed = true }
    }
    let limit = ProcessInfo.processInfo.systemUptime + 3
    while !completed && ProcessInfo.processInfo.systemUptime < limit {
      RunLoop.main.run(until: Date().addingTimeInterval(0.002))
    }
    precondition(completed && !failed)
    expectAction("preserve")
    print("backgroundHandshakeSerializesCleanupAndNotificationOnMainBeforeACK=true")
  }
}
'''
for marker, value in {
    "__TYPES__": types,
    "__MEMORY_BARRIER__": declaration(memory, "enum InputiaMemoryBarrier"),
    "__SCOPE__": scope,
    "__SYNCHRONIZE__": synchronize,
    "__APPLY__": apply,
}.items():
    fixture = fixture.replace(marker, value)

with tempfile.TemporaryDirectory(prefix="inputia-privacy-owned-context-") as directory:
    path = Path(directory)
    (path / "main.swift").write_text(fixture)
    subprocess.run(["swiftc", "-parse-as-library", str(path / "main.swift"), "-o", str(path / "self-check")], check=True)
    subprocess.run([str(path / "self-check")], check=True)

print("privacyOwnedContextProductionMethodsPassed=true noGUIOrUserProfileAccess=true")
