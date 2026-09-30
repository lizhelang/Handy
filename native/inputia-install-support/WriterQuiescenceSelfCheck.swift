import Darwin
import Foundation

private var assertions = 0
private func check(_ yes: Bool, _ label: String) {
  precondition(yes, label)
  assertions += 1
}
private func waitRunning(_ identity: KernelWriterIdentity) {
  let deadline = ProcessInfo.processInfo.systemUptime + 0.5
  while (try? kernelWriterIsStopped(identity)) == true,
    ProcessInfo.processInfo.systemUptime < deadline
  { usleep(10_000) }
}
private func rejects(_ label: String, _ work: () throws -> Void) {
  do {
    try work()
    preconditionFailure("unexpected acceptance: \(label)")
  } catch InstallCodeError.rejected { assertions += 1 } catch {
    preconditionFailure("untyped failure \(error)")
  }
}
@main
struct WriterQuiescenceSelfCheck {
  static func main() throws {
    // 唯一停止对象由本次测试直接创建；不枚举或停止日用 Inputia / IME。
    do {
      let slot = try WriterSuspensionSlot.acquire()
      rejects("overlapping local lease rejected") { _ = try WriterSuspensionSlot.acquire() }
      withExtendedLifetime(slot) {}
    }
    do {
      _ = try WriterSuspensionSlot.acquire()
      assertions += 1
    }
    let child = Process()
    child.executableURL = URL(fileURLWithPath: "/bin/sleep")
    child.arguments = ["30"]
    try child.run()
    defer {
      if child.isRunning {
        child.terminate()
        child.waitUntilExit()
      }
    }
    let identity = try captureKernelWriter(child.processIdentifier)
    check(identity.parent == UInt32(getpid()), "fixture child ownership")
    check(
      identity.uid == geteuid() && identity.pidVersion > 0 && identity.startSeconds > 0,
      "kernel identity complete")
    check(
      identity.path == "/bin/sleep" || identity.path == "/usr/bin/sleep", "actual executable path")
    check(try !kernelWriterHasExited(identity), "fixture initially running")
    let stale = KernelWriterIdentity(
      pid: identity.pid, uid: identity.uid, parent: identity.parent,
      startSeconds: identity.startSeconds + 1, startMicroseconds: identity.startMicroseconds,
      token: identity.token, path: identity.path)
    rejects("changed start time never signals") {
      try signalKernelWriter(stale, signal: SIGSTOP, authorize: { true })
    }
    check(child.isRunning, "stale proof did not stop child")
    rejects("self cannot be stopped") { _ = try captureKernelWriter(getpid()) }
    let expected = InstallCodeRequest(
      schema_version: 1,
      subject: .init(
        transaction_id: "11111111-1111-4111-8111-111111111111",
        plan_sha256: String(repeating: "a", count: 64),
        installation_id: "22222222-2222-4222-8222-222222222222", new_release_id: "inputia-new"),
      purpose: "previous_release", product_id: "com.inputia", role: "control",
      exact_bundle_path: "/synthetic/Inputia.app", bundle_id: "com.inputia.control",
      release_id: "inputia-old", version: "1.0.0", build: 1,
      source_commit: String(repeating: "b", count: 40), team_id: "TESTTEAM01",
      architectures: ["arm64"], cdhashes: [String(repeating: "c", count: 40)])
    rejects("Apple fixture is not an authorized Inputia writer") {
      _ = try verifyRunningWriter(child.processIdentifier, expected)
    }
    check(child.isRunning, "dynamic signature mismatch did not signal")
    let markerURL = FileManager.default.temporaryDirectory.appendingPathComponent(
      "inputia-marker-fixture-" + UUID().uuidString)
    let marker = Data("fixture-maintenance-epoch".utf8)
    try marker.write(to: markerURL)
    defer { try? FileManager.default.removeItem(at: markerURL) }
    let authorize = { (try? Data(contentsOf: markerURL)) == marker }
    // 授权在签名验证之后撤销：真实 STOP 前回调必须阻止效应。
    try FileManager.default.removeItem(at: markerURL)
    rejects("marker revoked after code verification blocks real STOP") {
      try signalKernelWriter(identity, signal: SIGSTOP, authorize: authorize)
    }
    check(try !kernelWriterIsStopped(identity), "revoked authority preserved running fixture")
    try marker.write(to: markerURL)
    let explicit = KernelWriterSuspension(observed: [identity])
    try explicit.suspend(authorize: authorize, validateTree: {})
    check(try kernelWriterIsStopped(identity), "lease holds real kernel stop")
    try FileManager.default.removeItem(at: markerURL)
    try explicit.resume()
    try explicit.resume()
    waitRunning(identity)
    check(
      try !kernelWriterIsStopped(identity),
      "explicit resume ignores revoked maintenance and is idempotent")
    rejects("resumed lease is not a suspension proof") { try explicit.assertSuspended() }
    do {
      let automatic = KernelWriterSuspension(observed: [identity])
      try automatic.suspend(authorize: { true }, validateTree: {})
      check(try kernelWriterIsStopped(identity), "Drop fixture held")
      withExtendedLifetime(automatic) {}
    }
    waitRunning(identity)
    check(try !kernelWriterIsStopped(identity), "Drop resumes original audit instance")
    try signalKernelWriter(identity, signal: SIGSTOP, authorize: { true })
    try waitStopped(identity)
    do {
      let alreadyStopped = KernelWriterSuspension(observed: [identity])
      try alreadyStopped.suspend(authorize: { true }, validateTree: {})
      try alreadyStopped.resume()
    }
    check(
      try kernelWriterIsStopped(identity),
      "preexisting stop is preserved on explicit resume and Drop")
    try signalKernelWriter(identity, signal: SIGCONT, authorize: { true })
    waitRunning(identity)
    let cancelled = KernelWriterSuspension(observed: [identity])
    var checks = 0
    rejects("revocation after STOP triggers rollback") {
      try cancelled.suspend(
        authorize: {
          checks += 1
          return checks == 1
        }, validateTree: {})
    }
    waitRunning(identity)
    check(try !kernelWriterIsStopped(identity), "rollback restores stopped fixture")
    // cleanup 只结束本fixture；生产 primitive 没有 TERM/KILL 功能。
    child.terminate()
    child.waitUntilExit()
    check(try kernelWriterHasExited(identity), "original fixture exited")
    rejects("old identity cannot signal again") {
      try signalKernelWriter(identity, signal: SIGSTOP, authorize: { true })
    }
    let parent = Process()
    let pipe = Pipe()
    parent.executableURL = URL(fileURLWithPath: "/bin/sh")
    parent.arguments = ["-c", "/bin/sleep 30 & echo $!; wait"]
    parent.standardOutput = pipe
    try parent.run()
    var childPIDText = Data()
    while true {
      let byte = pipe.fileHandleForReading.readData(ofLength: 1)
      guard !byte.isEmpty else { throw InstallCodeError.rejected("fixture_child_missing") }
      if byte == Data([10]) { break }
      childPIDText.append(byte)
    }
    guard let text = String(data: childPIDText, encoding: .utf8), let childPID = Int32(text) else {
      throw InstallCodeError.rejected("fixture_child_missing")
    }
    let parentIdentity = try captureKernelWriter(parent.processIdentifier)
    let descendant = try captureKernelWriter(childPID)
    defer {
      if let running = try? captureKernelWriter(descendant.pid),
        running.pidVersion == descendant.pidVersion
      {
        _ = kill(descendant.pid, SIGTERM)  // 仅清理本测试刚创建并复核的子进程。
      }
      if parent.isRunning {
        parent.terminate()
        parent.waitUntilExit()
      }
    }
    check(descendant.parent == UInt32(parentIdentity.pid), "real fixture parent child relationship")
    let tree = [parentIdentity, descendant].map {
      WriterTreeNode(
        pid: $0.pid, parent: $0.parent, uid: $0.uid, startSeconds: $0.startSeconds,
        startMicroseconds: $0.startMicroseconds)
    }
    rejects("external helper blocks writer proof") {
      try assertVerifiedWriterDescendants([parentIdentity], inventory: tree)
    }
    check(try !kernelWriterHasExited(parentIdentity), "unknown descendant does not kill parent")
    check(try !kernelWriterHasExited(descendant), "unknown descendant never signalled")
    try assertVerifiedWriterDescendants([parentIdentity, descendant], inventory: tree)
    assertions += 1
    var treeChecks = 0
    let rejectedLease = KernelWriterSuspension(observed: [parentIdentity])
    rejects("post-stop descendant discovery rolls back owned parent") {
      try rejectedLease.suspend(authorize: { true }) {
        treeChecks += 1
        if treeChecks == 2 {
          try assertVerifiedWriterDescendants([parentIdentity], inventory: tree)
        }
      }
    }
    waitRunning(parentIdentity)
    check(try !kernelWriterIsStopped(parentIdentity), "unknown child failure resumes parent")
    check(try !kernelWriterIsStopped(descendant), "unknown child is never stopped")
    _ = kill(descendant.pid, SIGTERM)  // 此PID是本次直接创建且仍被父fixture持有的子进程。
    parent.waitUntilExit()
    let subject = InstallSubject(
      transaction_id: "11111111-1111-4111-8111-111111111111",
      plan_sha256: String(repeating: "a", count: 64),
      installation_id: "22222222-2222-4222-8222-222222222222", new_release_id: "inputia-new")
    rejects("empty role coverage") {
      try validateWriterRequest(
        .init(
          schema_version: 1, action: "suspend", subject: subject,
          epoch: "33333333-3333-4333-8333-333333333333", old_release_id: "inputia-old", roles: []))
    }
    let reply = iuisWriterSuspend(nil, 1, nil, nil, nil)!
    check(String(cString: reply).contains("invalid_request"), "null C ABI input rejected")
    iuisStringFree(reply)
    print("WriterQuiescenceSelfCheck: \(assertions) assertions passed; own fixture process only")
  }
}
