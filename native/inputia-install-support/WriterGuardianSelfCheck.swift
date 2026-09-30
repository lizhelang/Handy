import Darwin
import Foundation

private var assertions = 0
private func check(_ value: Bool, _ label: String) {
  precondition(value, label)
  assertions += 1
}
private func rejects(_ label: String, _ work: () throws -> Void) {
  do {
    try work()
    preconditionFailure("unexpected acceptance: \(label)")
  } catch InstallCodeError.rejected { assertions += 1 } catch {
    preconditionFailure("untyped failure: \(error)")
  }
}
private func fixture(_ kernel: KernelWriterIdentity) -> VerifiedRunningWriter {
  // 合成角色metadata只在此自检中；效应使用真实内核实例。生产factory没有此入口。
  .init(
    kernel: kernel,
    evidence: .init(
      pid: kernel.pid, uid: kernel.uid, start_seconds: kernel.startSeconds,
      start_microseconds: kernel.startMicroseconds, pid_version: kernel.pidVersion,
      role: "control", bundle_id: "com.inputia.fixture", release_id: "inputia-fixture",
      executable_path: kernel.path, cdhash: String(repeating: "b", count: 40)))
}
@main
struct WriterGuardianSelfCheck {
  static func main() throws {
    let child = Process()
    child.executableURL = URL(fileURLWithPath: "/bin/sleep")
    child.arguments = ["30"]
    try child.run()
    let kernel = try captureKernelWriter(child.processIdentifier)
    defer {
      // 只清理本次直接spawn并持有的fixture，绝不扫描日用角色。
      try? signalKernelWriter(kernel, signal: SIGCONT, authorize: { true })
      if child.isRunning {
        child.terminate()
        child.waitUntilExit()
      }
    }
    do {
      let scoped = try GuardianNativePlan(
        writers: [fixture(kernel)], request: nil,
        executable: "fixture", slot: WriterSuspensionSlot.acquire())
      rejects("second guardian from same transaction cannot acquire shared slot") {
        _ = try GuardianNativePlan(
          writers: [fixture(kernel)], request: nil,
          executable: "fixture", slot: WriterSuspensionSlot.acquire())
      }
      rejects("legacy suspension cannot overlap guardian") {
        _ = try WriterSuspensionSlot.acquire()
      }
      scoped.closeEffects()
      _ = try scoped.resume(0)
      withExtendedLifetime(scoped) {}
    }
    do {
      let legacySlot = try WriterSuspensionSlot.acquire()
      rejects("guardian cannot overlap legacy suspension") {
        _ = try GuardianNativePlan(
          writers: [fixture(kernel)], request: nil,
          executable: "fixture", slot: WriterSuspensionSlot.acquire())
      }
      withExtendedLifetime(legacySlot) {}
    }
    do {
      _ = try WriterSuspensionSlot.acquire()
      assertions += 1
    }
    let parent = try GuardianNativePlan(
      writers: [fixture(kernel)], request: nil, executable: "fixture")
    let guardian = try GuardianNativePlan(
      writers: [fixture(kernel)], request: nil, executable: "fixture")
    check(parent.entries == guardian.entries, "independent plans agree before ARM")
    rejects("resume cannot precede irreversible close") { _ = try parent.resume(0) }
    try guardian.stop(0, authorize: { true })
    check(try guardian.state(0) == "stopped", "native STOP applies to real instance")
    try guardian.assertHolding()
    assertions += 1
    rejects("effect cannot repeat") { try guardian.stop(0, authorize: { true }) }
    // 与guardian死后的父备份相同：恢复依据双方ARM而非本handle是否执行过STOP。
    parent.closeEffects()
    check(try parent.resume(0) == "resumed", "independent backup can recover armed unknown effect")
    check(try parent.resume(0) == "running", "recovery is idempotent")
    rejects("closed backup never reopens STOP") { try parent.stop(0, authorize: { true }) }
    guardian.closeEffects()
    check(try guardian.resume(0) == "running", "old owner observes already recovered state")
    try signalKernelWriter(kernel, signal: SIGSTOP, authorize: { true })
    try waitStopped(kernel)
    let observed = try GuardianNativePlan(
      writers: [fixture(kernel)], request: nil, executable: "fixture")
    check(observed.entries[0].initialState == "observed_stopped", "original stop recorded")
    observed.closeEffects()
    rejects("original stopped cannot be resumed") { _ = try observed.resume(0) }
    check(try kernelWriterIsStopped(kernel), "original stop preserved")
    try signalKernelWriter(kernel, signal: SIGCONT, authorize: { true })
    let revoked = try GuardianNativePlan(
      writers: [fixture(kernel)], request: nil, executable: "fixture")
    rejects("last effect authorization denies STOP") { try revoked.stop(0, authorize: { false }) }
    check(try revoked.state(0) == "running", "revoked marker causes no effect")
    revoked.closeEffects()
    check(try revoked.resume(0) == "running", "ARM unknown with no effect is safe")
    rejects("index cannot name another process") { _ = try revoked.state(1) }
    let staleKernel = KernelWriterIdentity(
      pid: kernel.pid, uid: kernel.uid, parent: kernel.parent,
      startSeconds: kernel.startSeconds + 1, startMicroseconds: kernel.startMicroseconds,
      token: kernel.token, path: kernel.path)
    rejects("stale identity cannot create a plan") {
      _ = try GuardianNativePlan(
        writers: [fixture(staleKernel)], request: nil, executable: "fixture")
    }
    var handle: UnsafeMutableRawPointer?
    let reply = iuisGuardianPrepare(nil, 1, &handle)!
    check(
      handle == nil && String(cString: reply).contains("invalid_request"),
      "invalid ABI cannot manufacture capability")
    iuisStringFree(reply)
    child.terminate()
    child.waitUntilExit()
    check(try revoked.state(0) == "original_exited", "original exit is terminal metadata")
    check(try revoked.resume(0) == "original_exited", "exited instance is never signalled")
    print(
      "WriterGuardianSelfCheck: \(assertions) assertions passed; own fixture only; Developer ID positive NOT_RUN"
    )
  }
}
