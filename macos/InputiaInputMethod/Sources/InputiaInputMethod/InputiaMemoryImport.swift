import Foundation
import Darwin

/// 这里只保存可恢复的导入意图，不保存历史正文或数据库路径。
struct InputiaMemoryImportPending: Codable, Equatable {
  let format_version: Int
  let profile_id: String
  let server_instance: String
  let policy_epoch: UInt64
  let operation_id: String
  let selection: String
  let limit: Int
  func validate(profile: String) throws {
    guard format_version == 1, profile_id == profile, policy_epoch > 0,
      UUID(uuidString: operation_id) != nil, !server_instance.isEmpty, server_instance.utf8.count <= 128,
      InputiaMemoryQuery.clean(server_instance), ["history", "clipboard", "both"].contains(selection),
      (1...2000).contains(limit) else { throw InputiaMemoryError.invalid }
  }
  func mayResubmit(after policy: InputiaMemoryPolicy) -> Bool {
    // 服务重启仍查同一durable operation；epoch改变则绝不按新策略重放旧来源。
    profile_id == policy.profile_id && policy_epoch == policy.policy_epoch
  }
}

/// 一个窗口持有固定 flock 直到本次 Outcome/Import 完成；其他窗口不能另起相同来源的操作。
final class InputiaMemoryImportStore {
  private let root: Int32
  private let lock: Int32
  private let profile: String
  private let sync: (Int32) -> Int32
  private let basename = "memory-import-pending-v1.json"
  init(directory: URL, profile: String, sync: @escaping (Int32) -> Int32 = Darwin.fsync) throws {
    self.profile = profile
    self.sync = sync
    guard directory.path.hasPrefix("/"), !directory.path.contains("//"),
      !directory.path.split(separator: "/").contains(where: { $0 == "." || $0 == ".." }),
      InputiaMemoryQuery.clean(directory.path), !profile.isEmpty else { throw InputiaMemoryError.invalid }
    var directoryFD = Darwin.open("/", O_RDONLY | O_DIRECTORY | O_CLOEXEC)
    guard directoryFD >= 0 else { throw InputiaMemoryError.unavailable }
    do {
      for part in directory.path.split(separator: "/") {
        let next = String(part).withCString { openat(directoryFD, $0, O_RDONLY | O_DIRECTORY | O_CLOEXEC | O_NOFOLLOW) }
        guard next >= 0 else { throw InputiaMemoryError.unavailable }
        var metadata = stat()
        guard fstat(next, &metadata) == 0, metadata.st_mode & S_IFMT == S_IFDIR,
          metadata.st_uid == 0 || metadata.st_uid == geteuid(),
          metadata.st_mode & 0o022 == 0 || (metadata.st_uid == 0 && metadata.st_mode & 0o1000 != 0) else {
          close(next); throw InputiaMemoryError.invalid
        }
        close(directoryFD); directoryFD = next
      }
      var metadata = stat()
      guard fstat(directoryFD, &metadata) == 0, metadata.st_uid == geteuid(), metadata.st_mode & 0o077 == 0 else { throw InputiaMemoryError.invalid }
      let lockFD = openat(directoryFD, "memory-import-v1.lock", O_RDWR | O_CREAT | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK, 0o600)
      guard lockFD >= 0 else { throw InputiaMemoryError.unavailable }
      do {
        try Self.validateFile(lockFD)
        guard flock(lockFD, LOCK_EX | LOCK_NB) == 0 else { throw InputiaMemoryError.unavailable }
      } catch { close(lockFD); throw error }
      root = directoryFD; lock = lockFD
    } catch { close(directoryFD); throw error }
  }
  deinit { flock(lock, LOCK_UN); close(lock); close(root) }
  private static func validateFile(_ fd: Int32) throws {
    var metadata = stat()
    guard fstat(fd, &metadata) == 0, metadata.st_mode & S_IFMT == S_IFREG,
      metadata.st_uid == geteuid(), metadata.st_nlink == 1, metadata.st_mode & 0o7777 == 0o600,
      metadata.st_size >= 0, metadata.st_size <= 8192 else { throw InputiaMemoryError.invalid }
  }
  private static func encoded(_ pending: InputiaMemoryImportPending) throws -> Data {
    let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
    return try encoder.encode(pending)
  }
  func read() throws -> InputiaMemoryImportPending? {
    let fd = openat(root, basename, O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK)
    if fd < 0 { if errno == ENOENT { return nil }; throw InputiaMemoryError.invalid }
    defer { close(fd) }
    try Self.validateFile(fd)
    return try readDescriptor(fd)
  }
  private func readDescriptor(_ fd: Int32) throws -> InputiaMemoryImportPending {
    guard lseek(fd, 0, SEEK_SET) == 0 else { throw InputiaMemoryError.unavailable }
    var data = Data(), buffer = [UInt8](repeating: 0, count: 8193)
    while data.count <= 8192 {
      let count = Darwin.read(fd, &buffer, min(buffer.count, 8193 - data.count))
      if count == 0 { break }
      if count < 0 { if errno == EINTR { continue }; throw InputiaMemoryError.unavailable }
      data.append(contentsOf: buffer.prefix(count))
    }
    guard data.count <= 8192 else { throw InputiaMemoryError.invalid }
    let value = try JSONDecoder().decode(InputiaMemoryImportPending.self, from: data)
    try value.validate(profile: profile)
    // 自有记录采用固定编码；未知/重复字段与被修改的非规范记录不能悄悄恢复。
    guard try Self.encoded(value) == data else { throw InputiaMemoryError.invalid }
    return value
  }
  /// rename 后目录同步失败属于未知结果；恢复发送任何请求前必须重做耐久确认。
  func confirmDurable(_ expected: InputiaMemoryImportPending) throws {
    let fd = openat(root, basename, O_RDONLY | O_CLOEXEC | O_NOFOLLOW | O_NONBLOCK)
    guard fd >= 0 else { throw InputiaMemoryError.uncertain }
    defer { close(fd) }
    try Self.validateFile(fd)
    guard try readDescriptor(fd) == expected else { throw InputiaMemoryError.uncertain }
    guard sync(fd) == 0, sync(root) == 0 else { throw InputiaMemoryError.uncertain }
    var held = stat(), current = stat()
    guard fstat(fd, &held) == 0, fstatat(root, basename, &current, AT_SYMLINK_NOFOLLOW) == 0,
      held.st_dev == current.st_dev, held.st_ino == current.st_ino,
      try readDescriptor(fd) == expected else { throw InputiaMemoryError.uncertain }
    try Self.validateFile(fd)
  }
  func saveNew(_ pending: InputiaMemoryImportPending) throws {
    guard try read() == nil else { throw InputiaMemoryError.uncertain }
    try pending.validate(profile: profile)
    let data = try Self.encoded(pending), temporary = ".memory-import-\(UUID().uuidString).tmp"
    let fd = openat(root, temporary, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
    guard fd >= 0 else { throw InputiaMemoryError.unavailable }
    defer { close(fd); unlinkat(root, temporary, 0) }
    try data.withUnsafeBytes { bytes in
      var offset = 0
      while offset < bytes.count {
        let count = Darwin.write(fd, bytes.baseAddress!.advanced(by: offset), bytes.count - offset)
        if count < 0 && errno == EINTR { continue }
        guard count > 0 else { throw InputiaMemoryError.uncertain }; offset += count
      }
    }
    guard sync(fd) == 0 else { throw InputiaMemoryError.uncertain }
    // RENAME_EXCL 不能覆盖另一写者或攻击者刚出现的文件。
    guard renameatx_np(root, temporary, root, basename, UInt32(RENAME_EXCL)) == 0,
      sync(root) == 0 else { throw InputiaMemoryError.uncertain }
  }
  func complete(_ expected: InputiaMemoryImportPending) throws {
    guard try read() == expected else { throw InputiaMemoryError.uncertain }
    guard unlinkat(root, basename, 0) == 0, sync(root) == 0 else { throw InputiaMemoryError.uncertain }
  }
}

#if INPUTIA_PAIRED_BUILD
enum InputiaMemoryImport {
  private static let queue = DispatchQueue(label: "Inputia.memory-import-metadata")
  static func perform(selection: String, completion: @escaping (String) -> Void) {
    InputiaVoiceBridge.shared.prepare { prepared in
      guard case .success(let policy) = prepared else { completion("学习服务暂不可用；未创建新的导入操作"); return }
      queue.async {
        do {
          let profile = InputiaProfile.current
          guard policy.profile_id == profile.profileID else { throw InputiaMemoryError.invalid }
          let store = try InputiaMemoryImportStore(directory: profile.root, profile: profile.profileID)
          let previous = try store.read()
          let pending = previous ?? InputiaMemoryImportPending(format_version: 1, profile_id: policy.profile_id,
            server_instance: policy.server_instance, policy_epoch: policy.policy_epoch,
            operation_id: UUID().uuidString, selection: selection, limit: 2000)
          if previous == nil { try store.saveNew(pending) }
          try store.confirmDurable(pending)
          resume(store: store, pending: pending, policy: policy, completion: completion)
        } catch { DispatchQueue.main.async { completion("导入记录暂不可用或有未确认操作；已保留原记录") } }
      }
    }
  }
  private static func resume(store: InputiaMemoryImportStore, pending: InputiaMemoryImportPending,
                             policy: InputiaMemoryPolicy, completion: @escaping (String) -> Void) {
    InputiaVoiceBridge.shared.management(.init(kind: "outcome", operation_id: pending.operation_id), expectedEpoch: pending.policy_epoch) { reply in
      guard case .success(let reply) = reply, reply.code == nil, reply.result?.kind == "outcome" else {
        completion("未能确认导入结果；再次点击将查询同一操作"); return
      }
      if let operation = reply.result?.operation {
        finish(store: store, pending: pending, operation: operation, completion: completion); return
      }
      guard pending.mayResubmit(after: policy) else {
        completion("原操作尚无结果且策略已改变；保留记录，需要确认恢复"); return
      }
      InputiaVoiceBridge.shared.management(.init(kind: "import", operation_id: pending.operation_id,
        selection: pending.selection, limit: pending.limit), expectedEpoch: pending.policy_epoch) { result in
        guard case .success(let reply) = result, reply.code == nil, reply.result?.kind == "import", let operation = reply.result?.operation else {
          completion("导入尚未确认；再次点击将查询同一操作"); return
        }
        finish(store: store, pending: pending, operation: operation, completion: completion)
      }
    }
  }
  private static func finish(store: InputiaMemoryImportStore, pending: InputiaMemoryImportPending,
                             operation: InputiaMemoryOperationPayload, completion: @escaping (String) -> Void) {
    queue.async {
      do {
        let status = try operation.imported()
        guard status.operation_id == pending.operation_id, status.applied_at_epoch == pending.policy_epoch,
          ["accepted", "processing", "completed", "partial_failure", "revoked"].contains(status.state) else { throw InputiaMemoryError.invalid }
        if status.terminal { try store.complete(pending) }
        let summary = "导入状态：\(status.state)；语音 \(status.history_imported)，剪贴板 \(status.clipboard_imported)，跳过 \(status.skipped)"
        DispatchQueue.main.async { completion(summary) }
      } catch { DispatchQueue.main.async { completion("导入回执或持久化未确认；原操作记录已保留") } }
    }
  }
}
#endif
