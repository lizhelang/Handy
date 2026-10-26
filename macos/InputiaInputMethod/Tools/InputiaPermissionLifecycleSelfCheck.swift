import Foundation

private func allowed() -> InputiaPermissionProbe {
  .allowed(.init(server: "synthetic-service", epoch: 1,
    deadline: ProcessInfo.processInfo.systemUptime + 60))
}

@main
struct InputiaPermissionLifecycleSelfCheck {
  static func main() {
    var model = InputiaPermissionState()
    precondition(!model.permits(model.epoch))
    model.update(.ready)
    let first = model.epoch
    precondition(model.permits(first))
    model.update(.accessibilityRequired)
    precondition(!model.permits(first))
    model.update(.ready)
    precondition(!model.permits(first))
    let second = model.epoch
    model.update(.maintenance, markerEpoch: "one")
    model.update(.maintenance, markerEpoch: "two")
    model.update(.ready, markerEpoch: "two")
    precondition(!model.permits(second))

    let lock = NSLock()
    var calls = 0
    let release = DispatchSemaphore(value: 0)
    let checker = InputiaPermissionLifecycle(timeout: 0.05) {
      lock.lock(); calls += 1; lock.unlock()
      release.wait()
      return allowed()
    }
    checker.start(root: nil)
    Thread.sleep(forTimeInterval: 1.15)
    precondition(checker.snapshot.status == .unknown)
    lock.lock(); let observed = calls; lock.unlock()
    precondition(observed == 1, "hung query must not fan out")
    release.signal()
    Thread.sleep(forTimeInterval: 0.1)
    precondition(checker.snapshot.status == .unknown, "late permission result must not recover")
    checker.stop()

    let good = InputiaPermissionLifecycle(timeout: 0.2, query: { allowed() })
    good.start(root: nil)
    Thread.sleep(forTimeInterval: 0.1)
    precondition(good.isReady)
    let readyEpoch = good.epoch
    good.stop()
    precondition(!good.permits(readyEpoch))
    let root = FileManager.default.temporaryDirectory.appendingPathComponent("inputia-permission-test-" + UUID().uuidString)
    try! FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: root) }
    let marker = root.appendingPathComponent("permission-maintenance.json")
    try! Data("{\"schema_version\":1,\"active\":true,\"epoch\":\"test-maintenance\"}".utf8).write(to: marker)
    let maintained = InputiaPermissionLifecycle(timeout: 0.2, pollInterval: 0.05, query: { allowed() })
    maintained.start(root: root)
    Thread.sleep(forTimeInterval: 0.1)
    precondition(maintained.snapshot.status == .maintenance)
    precondition(!maintained.backgroundMaintenanceAllowsWork())
    try! FileManager.default.removeItem(at: marker)
    let markerTarget = root.appendingPathComponent("marker-target.json")
    try! Data("{\"schema_version\":1,\"active\":false,\"epoch\":\"fake\"}".utf8).write(to: markerTarget)
    try! FileManager.default.createSymbolicLink(at: marker, withDestinationURL: markerTarget)
    precondition(!maintained.backgroundMaintenanceAllowsWork(), "symlink markers must fail closed")
    try! FileManager.default.removeItem(at: marker)
    try! Data(repeating: 32, count: 16_385).write(to: marker)
    precondition(!maintained.backgroundMaintenanceAllowsWork(), "oversized markers must fail closed")
    let maintenanceEpoch = maintained.epoch
    try! FileManager.default.removeItem(at: marker)
    RunLoop.current.run(until: Date().addingTimeInterval(1.1))
    precondition(maintained.isReady && maintained.epoch != maintenanceEpoch)
    let health = root.appendingPathComponent("permission-health-ime.json")
    let attrs = try! FileManager.default.attributesOfItem(atPath: health.path)
    precondition((attrs[.posixPermissions] as? NSNumber)?.intValue == 0o600)
    let object = try! JSONSerialization.jsonObject(with: Data(contentsOf: health)) as! [String: Any]
    precondition(Set(object.keys) == Set(["schema_version", "component", "pid", "instance_id", "state", "permission_epoch", "maintenance_marker_epoch", "updated_at_ms"]))
    maintained.stop()
    Thread.sleep(forTimeInterval: 0.05)
    // Main callback and a queue behind an in-flight operation must both finish,
    // and even that ACK cannot hide a permission query that is still in flight.
    let ackRoot = FileManager.default.temporaryDirectory.appendingPathComponent("inputia-retirement-test-" + UUID().uuidString)
    try! FileManager.default.createDirectory(at: ackRoot, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: ackRoot) }
    let queryRelease = DispatchSemaphore(value: 0)
    let queryStarted = DispatchSemaphore(value: 0)
    let workerRelease = DispatchSemaphore(value: 0)
    let fakeIPC = DispatchQueue(label: "Inputia.test-blocked-ipc")
    fakeIPC.async { workerRelease.wait() }
    var cleanupQueued = false
    let retiring = InputiaPermissionLifecycle(timeout: 0.05) {
      queryStarted.signal(); queryRelease.wait(); return allowed()
    }
    retiring.start(root: ackRoot) { _, complete in
      cleanupQueued = true
      fakeIPC.async { DispatchQueue.main.async(execute: complete) }
    }
    precondition(queryStarted.wait(timeout: .now() + 1) == .success)
    let ackMarker = ackRoot.appendingPathComponent("permission-maintenance.json")
    try! Data("{\"schema_version\":1,\"active\":true,\"epoch\":\"retire-test\"}".utf8).write(to: ackMarker)
    Thread.sleep(forTimeInterval: 1.1)
    let ackHealth = ackRoot.appendingPathComponent("permission-health-ime.json")
    func healthState() -> String {
      let object = try! JSONSerialization.jsonObject(with: Data(contentsOf: ackHealth)) as! [String: Any]
      return object["state"] as! String
    }
    func pump(_ seconds: TimeInterval) { RunLoop.current.run(until: Date().addingTimeInterval(seconds)) }
    precondition(!cleanupQueued && healthState() == "retiring", "queued main cleanup cannot ACK maintenance")
    pump(0.1)
    precondition(cleanupQueued && healthState() == "retiring", "queued IPC cleanup cannot ACK maintenance")
    workerRelease.signal()
    pump(0.15)
    precondition(healthState() == "retiring", "completed cleanup cannot ACK a stuck AX query")
    queryRelease.signal()
    pump(0.15)
    precondition(healthState() == "maintenance", "all quiescence conditions must allow ACK")
    let acknowledged = try! JSONSerialization.jsonObject(with: Data(contentsOf: ackHealth)) as! [String: Any]
    precondition(acknowledged["maintenance_marker_epoch"] as? String == "retire-test")
    retiring.stop()
    pump(0.1)
    checkHardDeadlineAndRecovery()
    checkAuthenticatedServiceExpiry()
    checkServiceReplacementRequiresRetirementAndFreshProbe()
    checkStaleNotificationsAndProbeEvidence()
    print("permissionLifecycleSelfCheck=true transitions=true singleflight=true timeoutUnknown=true lateResultDiscarded=true shutdownInvalidates=true retirementACK=true")
  }

  private static func pumpUntil(_ condition: () -> Bool, timeout: TimeInterval = 1) {
    let deadline = ProcessInfo.processInfo.systemUptime + timeout
    while !condition(), ProcessInfo.processInfo.systemUptime < deadline {
      RunLoop.current.run(until: Date().addingTimeInterval(0.005))
    }
    precondition(condition(), "权限状态必须有界完成")
  }

  private static func checkServiceReplacementRequiresRetirementAndFreshProbe() {
    let lock = NSLock()
    var result = InputiaPermissionProbe.allowed(.init(server: "one", epoch: 1,
      deadline: ProcessInfo.processInfo.systemUptime + 60))
    var queries = 0
    let lifecycle = InputiaPermissionLifecycle(timeout: 0.2, pollInterval: 0.02) {
      lock.lock(); defer { lock.unlock() }; queries += 1; return result
    }
    func count() -> Int { lock.lock(); defer { lock.unlock() }; return queries }
    func set(_ value: InputiaPermissionProbe) { lock.lock(); result = value; lock.unlock() }
    var completions: [UInt64: () -> Void] = [:]
    lifecycle.start(root: nil) { transition, completed in
      if transition.requiresRetirement { completions[transition.epoch] = completed }
      else { completed() }
    }
    pumpUntil { lifecycle.isReady }
    var previousCompletion: (() -> Void)?
    for (server, remoteEpoch) in [("one", UInt64(2)), ("two", UInt64(2))] {
      let oldEpoch = lifecycle.epoch
      set(.allowed(.init(server: server, epoch: remoteEpoch,
        deadline: ProcessInfo.processInfo.systemUptime + 60)))
      pumpUntil { !lifecycle.isReady && completions[lifecycle.epoch] != nil }
      let retiringEpoch = lifecycle.epoch, before = count()
      precondition(!lifecycle.permits(oldEpoch))
      previousCompletion?()
      RunLoop.current.run(until: Date().addingTimeInterval(0.08))
      precondition(!lifecycle.isReady && count() == before,
        "清理ACK前不能启动探针，旧ACK不能结束新一代清理")
      let completed = completions[retiringEpoch]!
      completed()
      pumpUntil { lifecycle.isReady }
      precondition(count() > before && lifecycle.epoch != oldEpoch,
        "服务或远端代数换代只能用清理ACK之后启动的新探针恢复")
      precondition(lifecycle.matchesService(server: server, epoch: remoteEpoch))
      previousCompletion = completed
    }
    lifecycle.stop()
    print("serverAndRemoteEpochReplacementRetiresBeforeFreshProbe=true staleACKCannotReleaseNewRetirement=true")
  }

  private static func checkStaleNotificationsAndProbeEvidence() {
    let queued = InputiaPermissionLifecycle(timeout: 0.2, pollInterval: 0.02, query: { allowed() })
    var delivered: [InputiaPermissionState.Status] = []
    queued.start(root: nil) { transition, completed in delivered.append(transition.status); completed() }
    Thread.sleep(forTimeInterval: 0.06)
    precondition(queued.isReady)
    queued.stop()
    pumpUntil { !delivered.isEmpty }
    precondition(!delivered.contains(.ready), "主线程排队的旧ready通知必须失效")

    let lock = NSLock()
    let initialDeadline = ProcessInfo.processInfo.systemUptime + 0.8
    var result = InputiaPermissionProbe.allowed(.init(server: "current", epoch: 7, deadline: initialDeadline))
    var blockNext = false
    let entered = DispatchSemaphore(value: 0), release = DispatchSemaphore(value: 0)
    let lifecycle = InputiaPermissionLifecycle(timeout: 0.15, pollInterval: 0.02) {
      lock.lock(); let value = result, block = blockNext; blockNext = false; lock.unlock()
      if block { entered.signal(); release.wait() }
      return value
    }
    lifecycle.start(root: nil)
    pumpUntil { lifecycle.isReady }
    let originalEpoch = lifecycle.epoch
    lock.lock()
    result = .allowed(.init(server: "late", epoch: 99, deadline: ProcessInfo.processInfo.systemUptime + 60))
    blockNext = true
    lock.unlock()
    precondition(entered.wait(timeout: .now() + 1) == .success)
    lock.lock(); result = .unavailable(.transport); lock.unlock()
    RunLoop.current.run(until: Date().addingTimeInterval(0.2))
    release.signal()
    RunLoop.current.run(until: Date().addingTimeInterval(0.08))
    precondition(lifecycle.epoch == originalEpoch && lifecycle.matchesService(server: "current", epoch: 7),
      "迟到允许回执不得改变已认证服务身份或权限代数")
    pumpUntil({ !lifecycle.isReady }, timeout: 1)
    precondition(ProcessInfo.processInfo.systemUptime >= initialDeadline,
      "暂时不可得只可使用尚未过期的原证明")
    lifecycle.stop()
    print("staleReadyNotificationDiscarded=true lateServiceProofCannotChangeIdentity=true unavailableCannotRenewServiceDeadline=true")
  }

  private static func checkAuthenticatedServiceExpiry() {
    let lock = NSLock()
    var result: InputiaPermissionProbe = .allowed(.init(server: "service-one", epoch: 7,
      deadline: ProcessInfo.processInfo.systemUptime + 0.3))
    let lifecycle = InputiaPermissionLifecycle(timeout: 0.2, pollInterval: 0.05) {
      lock.lock(); defer { lock.unlock() }; return result
    }
    func set(_ value: InputiaPermissionProbe) { lock.lock(); result = value; lock.unlock() }
    lifecycle.start(root: nil)
    RunLoop.current.run(until: Date().addingTimeInterval(0.1))
    precondition(lifecycle.isReady)
    let old = lifecycle.epoch
    set(.unavailable(.transport))
    RunLoop.current.run(until: Date().addingTimeInterval(0.35))
    precondition(!lifecycle.permits(old), "暂时不可得不能延长已认证服务证明")
    set(.allowed(.init(server: "service-two", epoch: 7,
      deadline: ProcessInfo.processInfo.systemUptime + 2)))
    RunLoop.current.run(until: Date().addingTimeInterval(0.15))
    precondition(lifecycle.isReady && !lifecycle.permits(old), "新服务不能恢复旧插入票据")
    let restored = lifecycle.epoch
    set(.denied(server: "service-two", epoch: 8))
    RunLoop.current.run(until: Date().addingTimeInterval(0.1))
    precondition(!lifecycle.permits(restored), "明确拒权必须立即撤销已有回调")
    lifecycle.stop()
    print("serviceLeaseExpiry=true serverRestartInvalidates=true revokedCallbackRejected=true")
  }

  private static func checkHardDeadlineAndRecovery() {

  let lock=NSLock()
  var calls=0
  let stalled=DispatchSemaphore(value:0)
  let release=DispatchSemaphore(value:0)
  let recoveryStarted=DispatchSemaphore(value:0)
  let allowRecovery=DispatchSemaphore(value:0)
  let lifecycle=InputiaPermissionLifecycle(timeout:0.2) {
   lock.lock(); calls += 1;let n=calls;lock.unlock()
   if n == 2 {stalled.signal();release.wait()}
   if n == 3 {recoveryStarted.signal();allowRecovery.wait()}
   return allowed()
  }
  var notifications = 0
  lifecycle.start(root:nil) { _, complete in notifications += 1; complete() }
  RunLoop.current.run(until: Date().addingTimeInterval(0.15))
  precondition(lifecycle.isReady)
  let old=lifecycle.epoch
  precondition(stalled.wait(timeout:.now()+2) == .success)
  RunLoop.current.run(until: Date().addingTimeInterval(0.3))
  precondition(lifecycle.permits(old),"short delay must retain current permission")
  let beforeHardDeadline = notifications
  // 期间不访问snapshot/permits；只能由独立watchdog发出失效通知。
  RunLoop.current.run(until: Date().addingTimeInterval(4.1))
  precondition(notifications > beforeHardDeadline, "watchdog must notify without an accessor")
  let paused=lifecycle.snapshot
  print("hard_deadline_paused=\(paused.status == .unknown)")
  print("old_epoch_rejected=\(!lifecycle.permits(old))")
  guard paused.status == .unknown && !lifecycle.permits(old) else {
   lifecycle.stop();release.signal();allowRecovery.signal();exit(1)
  }
  lock.lock();precondition(calls == 2,"no replacement for hung query");lock.unlock()
  release.signal()
  precondition(recoveryStarted.wait(timeout:.now()+2) == .success)
  precondition(!lifecycle.isReady,"late reply must not reopen permission")
  allowRecovery.signal()
  Thread.sleep(forTimeInterval:0.1)
  precondition(lifecycle.isReady)
  precondition(!lifecycle.permits(old),"recovery must never replay pre-timeout operations")
  precondition(lifecycle.permits(lifecycle.epoch))
  lifecycle.stop()
  print("recovery_no_replay=true fresh_query_required=true singleflight=true autonomous_watchdog=true")
  }
}
