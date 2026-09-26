import Foundation

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
      return true
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

    let good = InputiaPermissionLifecycle(timeout: 0.2, query: { true })
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
    let maintained = InputiaPermissionLifecycle(timeout: 0.2, query: { true })
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
    Thread.sleep(forTimeInterval: 1.1)
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
      queryStarted.signal(); queryRelease.wait(); return true
    }
    retiring.start(root: ackRoot) { complete in
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
    print("permissionLifecycleSelfCheck=true transitions=true singleflight=true timeoutUnknown=true lateResultDiscarded=true shutdownInvalidates=true retirementACK=true")
  }

  private static func checkAuthenticatedServiceExpiry() {
    let lifecycle = InputiaPermissionLifecycle(timeout: 0.2, query: { true })
    lifecycle.observeService(server: "service-one", epoch: 7,
      deadline: ProcessInfo.processInfo.systemUptime + 0.3, ready: true)
    lifecycle.start(root: nil)
    RunLoop.current.run(until: Date().addingTimeInterval(0.1))
    precondition(lifecycle.isReady)
    let old = lifecycle.epoch
    RunLoop.current.run(until: Date().addingTimeInterval(0.35))
    precondition(!lifecycle.permits(old), "authenticated readiness expiry closes voice even if process lives")
    lifecycle.observeService(server: "service-two", epoch: 7,
      deadline: ProcessInfo.processInfo.systemUptime + 2, ready: true)
    RunLoop.current.run(until: Date().addingTimeInterval(0.65))
    precondition(lifecycle.isReady && !lifecycle.permits(old), "new server cannot revive old insertion callbacks")
    let restored = lifecycle.epoch
    lifecycle.observeService(server: "service-two", epoch: 8,
      deadline: ProcessInfo.processInfo.systemUptime + 2, ready: false)
    precondition(!lifecycle.permits(restored), "permission revocation rejects an already queued callback")
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
   return true
  }
  var notifications = 0
  lifecycle.start(root:nil) { complete in notifications += 1; complete() }
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
