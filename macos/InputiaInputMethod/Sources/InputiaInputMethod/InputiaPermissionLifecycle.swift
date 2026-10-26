import Foundation
import Darwin
import OSLog

enum InputiaPermissionProbe {
  struct Service {
    let server: String
    let epoch: UInt64
    let deadline: TimeInterval
  }
  enum Unavailable: String {
    case connectionGate = "connection_gate"
    case transport, staleReply = "stale_reply"
  }
  case allowed(Service)
  case denied(server: String, epoch: UInt64)
  case unavailable(Unavailable)
}

struct InputiaPermissionTransition {
  let epoch: UInt64
  let status: InputiaPermissionState.Status
  var requiresRetirement: Bool { status != .ready }
}

private enum InputiaPermissionDiagnostics {
  private static let log = Logger(subsystem: "com.inputia.permission", category: "probe")
  private static let lock = NSLock()
  private static var previousProbe: String?
  static func probe(_ result: InputiaPermissionProbe, elapsed: TimeInterval) {
    let reason: String
    switch result {
    case .allowed: reason = "allowed"
    case .denied: reason = "denied"
    case .unavailable(let unavailable): reason = unavailable.rawValue
    }
    lock.lock()
    let changed = previousProbe != reason
    previousProbe = reason
    lock.unlock()
    guard changed else { return }
    let milliseconds = Int(min(3_600_000, max(0, elapsed * 1000)))
    log.notice("result=\(reason, privacy: .public) elapsed_ms=\(milliseconds, privacy: .public)")
  }
  static func transition(_ value: InputiaPermissionTransition) {
    log.notice("state=\(value.status.rawValue, privacy: .public) epoch=\(value.epoch, privacy: .public) retirement=\(value.requiresRetirement, privacy: .public)")
  }
}

/// Pure transition model. Every permission boundary invalidates previously captured work.
struct InputiaPermissionState: Equatable {
  enum Status: String { case unknown, accessibilityRequired = "accessibility_required", maintenance, ready }
  private(set) var status: Status = .unknown
  private(set) var epoch: UInt64 = 0
  private(set) var markerEpoch: String?

  mutating func update(_ status: Status, markerEpoch: String? = nil) {
    if self.status != status || self.markerEpoch != markerEpoch {
      epoch &+= 1
      self.status = status
      self.markerEpoch = markerEpoch
    }
  }
  mutating func invalidate() { epoch &+= 1; status = .unknown }
  func permits(_ captured: UInt64) -> Bool { status == .ready && epoch == captured }
}

/// One AX query can remain blocked, but it never occupies main or spawns replacements.
/// A late result is discarded; the next completed, fresh query is required for recovery.
final class InputiaPermissionLifecycle {
  static let shared = InputiaPermissionLifecycle(pollInterval: 0.25)
  private let lock = NSLock()
  private let queue = DispatchQueue(label: "Inputia.permission-lifecycle")
  private let queryQueue = DispatchQueue(label: "Inputia.permission-query")
  private let watchdogQueue = DispatchQueue(label: "Inputia.permission-deadline")
  private var watchdog: DispatchSourceTimer?
  // 成功确认的有效期，而不是从最新一次查询开始计算；卡住的查询不能续期。
  private let hardTimeout: TimeInterval = 5.0
  private var model = InputiaPermissionState()
  private var checkedAt: TimeInterval = 0
  private var timer: DispatchSourceTimer?
  private var queryInFlight = false
  private var queryToken: UUID?
  private var stopped = false
  private var shutdown = false
  private var markerEpoch: String?
  private var maintenance = false
  private var root: URL?
  private var changed: ((InputiaPermissionTransition, @escaping () -> Void) -> Void)?
  // Written only on lifecycle queue, after main and all component queues acknowledge.
  private var acknowledgedEpoch: UInt64?
  private var retirementEpoch: UInt64?
  private var query: () -> InputiaPermissionProbe
  private let pollInterval: TimeInterval
  private let timeout: TimeInterval
  private let instance = UUID().uuidString

  init(timeout: TimeInterval = 0.75, pollInterval: TimeInterval = 1,
    query: @escaping () -> InputiaPermissionProbe = { .unavailable(.transport) }) {
    self.pollInterval = pollInterval
    self.timeout = timeout
    self.query = query
  }
  private var serviceIdentity: String?
  private var serviceDeadline: TimeInterval = .infinity
  func retireProbe(_ completion: @escaping () -> Void) { queryQueue.async(execute: completion) }
  func configureProbe(_ probe: @escaping () -> InputiaPermissionProbe) { queue.async { self.query = probe } }
  func isCurrentEpoch(_ epoch: UInt64) -> Bool { snapshot.epoch == epoch }
  var allowsServiceConnection: Bool { snapshot.status != .maintenance }
  func matchesService(server: String, epoch: UInt64) -> Bool {
    lock.lock(); defer { lock.unlock() }
    return serviceIdentity == "\(server):\(epoch)" && model.status == .ready && ProcessInfo.processInfo.systemUptime < serviceDeadline
  }
  var snapshot: InputiaPermissionState {
    lock.lock()
    var expired = false
    // 单次慢查询可短暂沿用确认结果；硬截止与是否仍有查询在途无关。
    if model.status == .ready
      && (ProcessInfo.processInfo.systemUptime - checkedAt >= hardTimeout || ProcessInfo.processInfo.systemUptime >= serviceDeadline) {
      model.update(.unknown, markerEpoch: model.markerEpoch)
      retirementEpoch = model.epoch
      expired = true
    }
    let result = model
    lock.unlock()
    if expired { notifyChanged(.init(epoch: result.epoch, status: result.status)) }
    return result
  }
  var epoch: UInt64 { snapshot.epoch }
  var isReady: Bool { snapshot.status == .ready }
  func permits(_ epoch: UInt64) -> Bool { snapshot.permits(epoch) }

  func start(root: URL?, changed: @escaping (InputiaPermissionTransition, @escaping () -> Void) -> Void = { _, complete in complete() }) {
    queue.async { [self] in
      guard self.timer == nil else { return }
      self.lock.lock()
      self.root = root
      self.shutdown = false
      self.lock.unlock()
      self.changed = changed
      self.stopped = false
      let timer = DispatchSource.makeTimerSource(queue: self.queue)
      timer.schedule(deadline: .now(), repeating: .milliseconds(Int(self.pollInterval * 1000)))
      timer.setEventHandler { self.refresh() }
      self.timer = timer
      timer.resume()
      // 独立队列只检查内存中的截止时间；不被AX调用或维护文件读取拖住。
      let watchdog = DispatchSource.makeTimerSource(queue: self.watchdogQueue)
      watchdog.schedule(deadline: .now(), repeating: .milliseconds(100))
      watchdog.setEventHandler { [weak self] in _ = self?.snapshot }
      self.watchdog = watchdog
      watchdog.resume()
    }
  }
  func stop() {
    // Immediate gate closure, with no IPC, filesystem, or TCC wait on the caller.
    lock.lock()
    shutdown = true
    model.invalidate()
    retirementEpoch = model.epoch
    let epoch = model.epoch
    lock.unlock()
    notifyChanged(.init(epoch: epoch, status: .unknown))
    queue.async {
      self.stopped = true
      self.timer?.cancel(); self.timer = nil
      self.watchdog?.cancel(); self.watchdog = nil
      self.queryToken = nil
      self.writeHealth()
    }
  }
  /// For launch readiness only, from a background queue: observe a just-written maintenance marker.
  func backgroundMaintenanceAllowsWork() -> Bool {
    lock.lock(); let configured = root != nil; let running = !shutdown; lock.unlock()
    return configured && running && !readMaintenance().0
  }

  private func readMaintenance() -> (Bool, String?) {
    lock.lock()
    let root = self.root
    lock.unlock()
    guard let root else { return (false, nil) }
    let path = root.appendingPathComponent("permission-maintenance.json")
    // One descriptor avoids a check/read race; nonblocking rejects a FIFO without hanging.
    let descriptor = open(path.path, O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC)
    guard descriptor >= 0 else { return errno == ENOENT ? (false, nil) : (true, "invalid") }
    defer { close(descriptor) }
    var info = stat()
    guard fstat(descriptor, &info) == 0, (info.st_mode & S_IFMT) == S_IFREG,
      info.st_size >= 0, info.st_size <= 16_384 else { return (true, "invalid") }
    var bytes = [UInt8](repeating: 0, count: 16_385)
    var count = 0
    while count < bytes.count {
      let amount = bytes.withUnsafeMutableBytes { buffer in
        read(descriptor, buffer.baseAddress!.advanced(by: count), buffer.count - count)
      }
      if amount < 0 {
        if errno == EINTR { continue }
        return (true, "invalid")
      }
      if amount == 0 { break }
      count += amount
    }
    guard count <= 16_384 else { return (true, "invalid") }
    let data = Data(bytes.prefix(count))
    guard
      let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
      object["schema_version"] as? Int == 1, let active = object["active"] as? Bool,
      let epoch = object["epoch"] as? String else { return (true, "invalid") }
    return (active, epoch)
  }
  private func refresh() {
    guard !stopped else { return }
    let marker = readMaintenance()
    if markerEpoch != marker.1 || maintenance != marker.0 {
      markerEpoch = marker.1; maintenance = marker.0
      queryToken = nil
      publish(maintenance ? .maintenance : .unknown, marker: markerEpoch)
    }
    if maintenance {
      publish(.maintenance, marker: markerEpoch)
      writeHealth(); return
    }
    let requestEpoch = snapshot.epoch
    lock.lock()
    let retiring = retirementEpoch != nil
    lock.unlock()
    // 清理前启动的证明不能在 ACK 到达后用于恢复；必须由下一次查询重新证明。
    guard !queryInFlight, !retiring else { writeHealth(); return }
    queryInFlight = true
    let token = UUID()
    queryToken = token
    let started = ProcessInfo.processInfo.systemUptime
    queue.asyncAfter(deadline: .now() + timeout) {
      guard self.queryToken == token, !self.stopped else { return }
      self.queryToken = nil
      // 软超时不立即打断已确认的会话，但snapshot始终执行硬截止。
      if self.snapshot.status != .ready {
        self.publish(.unknown, marker: self.markerEpoch)
      }
      self.writeHealth()
    }
    queryQueue.async {
      let result = self.query()
      InputiaPermissionDiagnostics.probe(result, elapsed: ProcessInfo.processInfo.systemUptime - started)
      self.queue.async {
        self.queryInFlight = false
        defer { self.writeHealth() }
        guard self.queryToken == token, !self.stopped, !self.maintenance,
          self.snapshot.epoch == requestEpoch,
          ProcessInfo.processInfo.systemUptime - started < self.timeout else { return }
        self.queryToken = nil
        self.acceptProbe(result, requestEpoch: requestEpoch, started: started)
        self.writeHealth()
      }
    }
  }
  // 与硬截止失效共用同一把锁，避免“先检查旧epoch、随后过期、再接受旧结果”的竞争。
  private func acceptProbe(_ result: InputiaPermissionProbe, requestEpoch: UInt64, started: TimeInterval) {
    lock.lock()
    let previous = model
    let now = ProcessInfo.processInfo.systemUptime
    if model.status == .ready && (now - checkedAt >= hardTimeout || now >= serviceDeadline) {
      model.update(.unknown, markerEpoch: model.markerEpoch)
      retirementEpoch = model.epoch
    }
    if !shutdown && model.epoch == requestEpoch && retirementEpoch == nil && now - started < timeout {
      switch result {
      case .allowed(let service) where now < service.deadline:
        let identity = "\(service.server):\(service.epoch)"
        let replacesReadyService = model.status == .ready && serviceIdentity != identity
        serviceIdentity = identity
        serviceDeadline = service.deadline
        if replacesReadyService {
          model.update(.unknown, markerEpoch: markerEpoch)
          retirementEpoch = model.epoch
        } else {
          model.update(.ready, markerEpoch: markerEpoch)
          checkedAt = now
        }
      case .denied(let server, let epoch):
        serviceIdentity = "\(server):\(epoch)"
        serviceDeadline = now
        model.update(.accessibilityRequired, markerEpoch: markerEpoch)
        if model != previous { retirementEpoch = model.epoch }
      case .allowed, .unavailable:
        // 没有新鲜证明不等于明确拒权，也不能续期已有证明。
        break
      }
    }
    let didChange = model != previous
    let transition = InputiaPermissionTransition(epoch: model.epoch, status: model.status)
    lock.unlock()
    if didChange { notifyChanged(transition) }
  }
  private func publish(_ status: InputiaPermissionState.Status, marker: String? = nil) {
    lock.lock()
    let previous = model
    model.update(status, markerEpoch: marker)
    let didChange = model != previous
    if didChange && status != .ready { retirementEpoch = model.epoch }
    let transition = InputiaPermissionTransition(epoch: model.epoch, status: model.status)
    lock.unlock()
    if didChange { notifyChanged(transition) }
  }
  private func notifyChanged(_ transition: InputiaPermissionTransition) {
    InputiaPermissionDiagnostics.transition(transition)
    DispatchQueue.main.async {
      guard self.isCurrentEpoch(transition.epoch), self.snapshot.status == transition.status else { return }
      let completed = {
        self.queue.async {
          // Stale cleanup cannot acknowledge a newer permission boundary.
          self.lock.lock()
          guard self.model.epoch == transition.epoch else { self.lock.unlock(); return }
          self.acknowledgedEpoch = transition.epoch
          if self.retirementEpoch == transition.epoch { self.retirementEpoch = nil }
          self.lock.unlock()
          self.writeHealth()
        }
      }
      if let changed = self.changed { changed(transition, completed) } else { completed() }
    }
  }
  private func writeHealth() {
    guard let root else { return }
    let state = snapshot
    // Gate closure is immediate; maintenance is an ACK, not merely a request.
    lock.lock()
    let acknowledged = acknowledgedEpoch == state.epoch && retirementEpoch == nil
    lock.unlock()
    let retired = acknowledged && !queryInFlight
    let healthState = state.status == .maintenance && !retired ? "retiring" : state.status.rawValue
    let health: [String: Any] = ["schema_version": 1, "component": "ime",
      "pid": ProcessInfo.processInfo.processIdentifier, "instance_id": instance,
      "state": healthState, "permission_epoch": state.epoch,
      "maintenance_marker_epoch": state.markerEpoch ?? "",
      "updated_at_ms": UInt64(Date().timeIntervalSince1970 * 1000)]
    guard let data = try? JSONSerialization.data(withJSONObject: health) else { return }
    let temporary = root.appendingPathComponent(".permission-health-ime-\(instance).tmp")
    let destination = root.appendingPathComponent("permission-health-ime.json")
    do {
      try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
      guard FileManager.default.createFile(atPath: temporary.path, contents: nil,
        attributes: [.posixPermissions: 0o600]) else { return }
      try data.write(to: temporary)
      guard rename(temporary.path, destination.path) == 0 else { throw CocoaError(.fileWriteUnknown) }
    } catch { try? FileManager.default.removeItem(at: temporary) }
  }
}
