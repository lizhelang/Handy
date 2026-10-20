#!/usr/bin/env python3
"""执行生产探针、连接门、权限生命周期、主线程通知及分队列退休；仅替换认证传输边界。"""
import argparse
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser()
parser.add_argument('--source-directory', type=Path, default=root / 'Sources/InputiaInputMethod')
parser.add_argument('--scenario', choices=['all', 'gate'], default='all')
args = parser.parse_args()
launcher = (args.source_directory / 'InputiaVoiceInputLauncher.swift').read_text()
host = (args.source_directory / 'main.swift').read_text()

def declaration(text, marker):
    start = text.index(marker)
    if 'func ' in marker or '.start(root:' in marker:
        position, parentheses = text.index('(', start) + 1, 1
        while parentheses:
            parentheses += (text[position] == '(') - (text[position] == ')')
            position += 1
        brace = text.index('{', position)
    else:
        brace = text.index('{', start)
    end, depth = brace + 1, 1
    while depth:
        depth += (text[end] == '{') - (text[end] == '}')
        end += 1
    return text[start:end].replace('fileprivate ', '').replace('private ', '')

parts = {
    '__GATE__': declaration(launcher, 'final class InputiaConnectionRetryGate'),
    '__PROBE__': declaration(launcher, '  static func probeServicePermission()'),
    '__RETIRE__': declaration(launcher, '  static func invalidatePermissionWork('),
    '__OPEN__': declaration(launcher, '  private static func openAuthenticatedConnection(profile:'),
    '__WIRING__': declaration(host, '      InputiaPermissionLifecycle.shared.start(root:'),
}
fixture = r'''
import Foundation
import Darwin
final class Facts {
  static let shared = Facts()
  let lock = NSLock()
  var counts: [String: Int] = [:]
  var failNextStatus = false, denied = false
  var server = "synthetic-service", remoteEpoch: UInt64 = 1
  func count(_ name: String) -> Int { lock.lock(); defer { lock.unlock() }; return counts[name, default: 0] }
  func add(_ name: String) { lock.lock(); counts[name, default: 0] += 1; lock.unlock() }
  func failOnce() { lock.lock(); failNextStatus = true; lock.unlock() }
  func reply() throws -> InputiaTargetBridgeReply {
    lock.lock(); defer { lock.unlock() }
    counts["status", default: 0] += 1
    if failNextStatus { failNextStatus = false; counts["transport", default: 0] += 1; throw InputiaVoiceServiceError.handshake }
    return .init(ready: !denied, server_instance: server, permission_epoch: remoteEpoch,
      deadline: ProcessInfo.processInfo.systemUptime + 1)
  }
}
enum InputiaVoiceServiceError: Error { case profile, handshake, policy }
struct InputiaTargetBridgeCommand { let kind: String }
struct InputiaTargetBridgeReply {
  let ready: Bool, server_instance: String
  let permission_epoch: UInt64
  let deadline: TimeInterval
}
struct Version { let policy_epoch: UInt64 = 1 }
struct Endpoint { let server_instance = "synthetic-service", socket_path = "/synthetic/no-socket" }
struct AuthenticatedInputs { let endpoint = Endpoint(), manifest = Data() }
struct InputiaProfile {
  static let current = InputiaProfile()
  let root = FileManager.default.temporaryDirectory.appendingPathComponent("inputia-probe-fixture-" + UUID().uuidString).appendingPathComponent("profile")
}
enum InputiaEmbeddedPairTrust { static let trust = 1 }
struct InputiaVoiceSharedState {
  init(profile: InputiaProfile) throws {}
  func lastVersion() -> Version { .init() }
}
final class InputiaVoiceServiceConnection {
  struct Server { let instance_id = "synthetic-service" }
  let server = Server()
  static func connect(endpoint: String, signedManifest: Data, trust: Int,
    profile: InputiaProfile, previousEpoch: UInt64) throws -> InputiaVoiceServiceConnection {
    Facts.shared.add("open"); return .init()
  }
  func synchronizePolicy(using state: InputiaVoiceSharedState) throws {}
  func targetBridge(_ command: InputiaTargetBridgeCommand) throws -> InputiaTargetBridgeReply { try Facts.shared.reply() }
  func close() { Facts.shared.add("close") }
  func closeTypedCaptureConnection() { close() }
}
struct BusinessConnection { let connection: InputiaVoiceServiceConnection }
struct InputiaVoiceFirstFetchGate {}
final class InputiaSharedTermsMemory {
  static let shared = InputiaSharedTermsMemory()
  func clear() { Facts.shared.add("clear") }
}
struct SyntheticTarget { let target_id = "synthetic-target" }
struct SyntheticTerms {}
struct SyntheticTrigger {}
final class SyntheticController {
  func synchronizePermissionEpoch() {}
  func shortcutRegistrationTarget() -> SyntheticTarget? { nil }
  func rejectShortcutRegistrationTarget(_ id: String) {}
  func acceptSharedTerms(_ terms: SyntheticTerms, ticket: UInt64) {}
  func acceptUnifiedShortcut(_ trigger: SyntheticTrigger, completion: (Bool) -> Void) { completion(false) }
}
enum InputiaHost {
  struct Controllers { let allObjects = [SyntheticController]() }
  static let inputControllers = Controllers()
  static var activeInputController: SyntheticController?
  static func removeGlobalMonitors() {}
  static func installGlobalMonitors() {}
}
__GATE__
enum InputiaVoiceInputLauncher {
  static let connectionRetryGate = InputiaConnectionRetryGate()
  static let personalizationQueue = DispatchQueue(label: "fixture.personalization")
  static let typedCaptureQueue = DispatchQueue(label: "fixture.typed")
  static let shortcutQueue = DispatchQueue(label: "fixture.shortcut")
  static let sharedTermsQueue = DispatchQueue(label: "fixture.shared")
  static let voiceQueue = DispatchQueue(label: "fixture.voice")
  static let targetQueue = DispatchQueue(label: "fixture.target")
  static let readinessQueue = DispatchQueue(label: "fixture.readiness")
  static let readinessLaunches = DispatchGroup()
  static var personalizationConnection: BusinessConnection?, typedCaptureConnection: BusinessConnection?
  static var permissionConnection: InputiaVoiceServiceConnection?, targetConnection: InputiaVoiceServiceConnection?
  static var shortcutConnection: InputiaVoiceServiceConnection?, sharedTermsConnection: InputiaVoiceServiceConnection?
  static var unifiedConnection: InputiaVoiceServiceConnection?, unifiedSession: String?
  static var shortcutTimer: DispatchSourceTimer?
  static var shortcutLease: String?, shortcutRejectedTargetID: String?, shortcutProviderHadTarget: Bool?
  static var shortcutLeaseEpoch: UInt64 = 0
  static var shortcutHasActiveOwner = false, shortcutCycleBusy = false
  static var shortcutSession: String?, shortcutTarget: String?, shortcutServer: String?, shortcutTerms: String?
  static var shortcutDeliver: String?, shortcutStatus: String?, lastUnifiedPhase: String?
  static var unifiedFetchGate = InputiaVoiceFirstFetchGate()
  static func ensureUnifiedServiceReady() {}
  static func startShortcutListening(targetProvider: () -> SyntheticTarget?, targetRejected: (String) -> Void,
    sharedTermsReceiver: (SyntheticTerms, UInt64) -> Void,
    acceptStart: (SyntheticTrigger, (Bool) -> Void) -> Void) { Facts.shared.add("listener_started") }
  static func openAuthenticatedConnection() throws -> InputiaVoiceServiceConnection {
    try openAuthenticatedConnection(profile: .current, inputs: AuthenticatedInputs())
  }
  __OPEN__
  __PROBE__
  __RETIRE__
}
func pump(_ seconds: Double) { RunLoop.main.run(until: Date().addingTimeInterval(seconds)) }
func until(_ check: () -> Bool, timeout: Double = 2) {
  let deadline = ProcessInfo.processInfo.systemUptime + timeout
  while !check() && ProcessInfo.processInfo.systemUptime < deadline { pump(0.005) }
  precondition(check(), "production fixture timed out")
}
@main struct Check {
  static func main() {
    let lifecycle = InputiaPermissionLifecycle.shared
    lifecycle.configureProbe { InputiaVoiceInputLauncher.probeServicePermission() }
    __WIRING__
    until { lifecycle.isReady && Facts.shared.count("listener_started") > 0 }
    pump(0.3)
    if CommandLine.arguments[1] != "gate" {
      precondition(Facts.shared.count("close") == 0, "ready通知不能关闭刚证明成功的探针连接")
      print("readyNotificationPreservesProbe=true")
    }
    let epoch = lifecycle.epoch, starts = Facts.shared.count("listener_started")
    let attempt = InputiaVoiceInputLauncher.connectionRetryGate.begin(now: ProcessInfo.processInfo.systemUptime)!
    Facts.shared.failOnce()
    until { Facts.shared.count("transport") == 1 }
    pump(0.28)
    precondition(lifecycle.isReady && lifecycle.epoch == epoch,
      "其他连接占用gate与暂时传输失败不能在旧证明有效期内冒充拒权")
    precondition(Facts.shared.count("listener_started") == starts, "暂时不可得不能反复重建监听")
    InputiaVoiceInputLauncher.connectionRetryGate.finish(attempt, succeeded: true, now: ProcessInfo.processInfo.systemUptime)
    until { Facts.shared.count("open") >= 2 && Facts.shared.count("status") >= 3 }
    precondition(lifecycle.isReady && lifecycle.epoch == epoch)
    print("productionProbeGateBusyRetainsFreshGrant=true noListenerChurn=true")
    Facts.shared.lock.lock(); Facts.shared.denied = true; Facts.shared.lock.unlock()
    until { !lifecycle.permits(epoch) }
    precondition(lifecycle.snapshot.status == .accessibilityRequired)
    let deniedEpoch = lifecycle.epoch
    Facts.shared.lock.lock(); Facts.shared.denied = false; Facts.shared.lock.unlock()
    until { lifecycle.isReady && lifecycle.epoch != deniedEpoch }
    precondition(!lifecycle.permits(epoch))
    print("authenticatedDenialClosesImmediately=true newProbeAfterRetirementRecovers=true")
    let closes = Facts.shared.count("close")
    var oldCleanupFinished = false
    #if !LEGACY_PROBE
    InputiaVoiceInputLauncher.invalidatePermissionWork(expectedEpoch: deniedEpoch) { oldCleanupFinished = true }
    until { oldCleanupFinished }
    precondition(Facts.shared.count("close") == closes, "旧退休请求不得关闭恢复后的连接")
    print("obsoleteInvalidationCannotCloseRecoveredConnections=true")
    let target = InputiaVoiceServiceConnection()
    InputiaVoiceInputLauncher.targetConnection = target
    let queuedEpoch = lifecycle.epoch
    let entered = DispatchSemaphore(value: 0), release = DispatchSemaphore(value: 0)
    let inspected = DispatchSemaphore(value: 0), finishInspection = DispatchSemaphore(value: 0)
    InputiaVoiceInputLauncher.targetQueue.async { entered.signal(); release.wait() }
    precondition(entered.wait(timeout: .now() + 1) == .success)
    InputiaVoiceInputLauncher.invalidatePermissionWork(expectedEpoch: queuedEpoch)
    InputiaVoiceInputLauncher.targetQueue.async {
      if InputiaVoiceInputLauncher.targetConnection === target { Facts.shared.add("stale_target_preserved") }
      inspected.signal(); finishInspection.wait()
    }
    lifecycle.stop()
    pump(0.03)
    release.signal()
    precondition(inspected.wait(timeout: .now() + 1) == .success)
    precondition(Facts.shared.count("stale_target_preserved") == 1,
      "已排队的旧epoch退休任务执行时必须复核归属，不能关闭新代连接")
    finishInspection.signal()
    until { InputiaVoiceInputLauncher.targetQueue.sync { InputiaVoiceInputLauncher.targetConnection == nil } }
    print("queuedStaleRetirementSkipsNewEpoch=true currentShutdownStillClosesConnections=true")
    #else
    lifecycle.stop()
    #endif
    pump(0.1)
  }
}
'''
for marker, value in parts.items():
    fixture = fixture.replace(marker, value)
with tempfile.TemporaryDirectory(prefix='inputia-probe-integration-') as temporary:
    folder = Path(temporary)
    path = folder / 'Fixture.swift'
    path.write_text(fixture)
    binary = folder / 'check'
    command = ['/usr/bin/swiftc', '-parse-as-library', str(path),
               str(args.source_directory / 'InputiaPermissionLifecycle.swift'), '-o', str(binary)]
    if '-> Bool' in parts['__PROBE__']:
        command += ['-D', 'LEGACY_PROBE']
    subprocess.run(command, check=True)
    raise SystemExit(subprocess.run([str(binary), args.scenario], check=False).returncode)
