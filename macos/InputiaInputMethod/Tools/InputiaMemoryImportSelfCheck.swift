import Foundation
import Darwin

@main
struct InputiaMemoryImportSelfCheck {
  static func main() throws {
    guard let temporary = realpath(NSTemporaryDirectory(), nil) else { throw InputiaMemoryError.unavailable }
    defer { free(temporary) }
    let root = URL(fileURLWithPath: String(cString: temporary))
      .appendingPathComponent("inputia-import-fixture-\(UUID().uuidString)")
    try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    defer { try? FileManager.default.removeItem(at: root) }
    var checks = 0
    func check(_ result: Bool) { precondition(result); checks += 1 }
    let pending = InputiaMemoryImportPending(format_version: 1, profile_id: "fixture", server_instance: "server",
      policy_epoch: 1, operation_id: UUID().uuidString, selection: "both", limit: 2000)
    check(pending.mayResubmit(after: .init(server_instance: "restarted-server", profile_id: "fixture", policy_epoch: 1)))
    check(!pending.mayResubmit(after: .init(server_instance: "server", profile_id: "fixture", policy_epoch: 2)))
    check(!pending.mayResubmit(after: .init(server_instance: "server", profile_id: "other", policy_epoch: 1)))
    var store: InputiaMemoryImportStore? = try InputiaMemoryImportStore(directory: root, profile: "fixture")
    check(try store!.read() == nil)
    check((try? InputiaMemoryImportStore(directory: root, profile: "fixture")) == nil)
    try store!.saveNew(pending); check(try store!.read() == pending)
    check((try? store!.saveNew(pending)) == nil)
    store = nil
    store = try InputiaMemoryImportStore(directory: root, profile: "fixture")
    check(try store!.read() == pending)
    let path = root.appendingPathComponent("memory-import-pending-v1.json")
    let original = try Data(contentsOf: path)
    let value = String(decoding: original, as: UTF8.self)
    for invalid in [" " + value, value.replacingOccurrences(of: "\"format_version\":1", with: "\"format_version\":1,\"format_version\":1"),
                    value.replacingOccurrences(of: "\"fixture\"", with: "\"other\"")] {
      try Data(invalid.utf8).write(to: path)
      check((try? store!.read()) == nil)
    }
    try original.write(to: path)
    chmod(path.path, 0o644); check((try? store!.read()) == nil); chmod(path.path, 0o600)
    let link = root.appendingPathComponent("alias.json")
    check(Darwin.link(path.path, link.path) == 0); check((try? store!.read()) == nil)
    try FileManager.default.removeItem(at: link)
    try store!.complete(pending); check(try store!.read() == nil)
    try FileManager.default.createSymbolicLink(at: path, withDestinationURL: link)
    check((try? store!.read()) == nil)
    store = nil
    let alias = root.appendingPathComponent("directory-link")
    try FileManager.default.createSymbolicLink(at: alias, withDestinationURL: root)
    check((try? InputiaMemoryImportStore(directory: alias, profile: "fixture")) == nil)
    let faultRoot = root.appendingPathComponent("fault")
    try FileManager.default.createDirectory(at: faultRoot, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
    var syncCalls = 0, failSync = true, sent = 0
    let faulty = try InputiaMemoryImportStore(directory: faultRoot, profile: "fixture") { fd in
      syncCalls += 1
      if failSync && syncCalls >= 2 { errno = EIO; return -1 }
      return Darwin.fsync(fd)
    }
    check((try? faulty.saveNew(pending)) == nil)
    check(try faulty.read() == pending) // rename 已发生，目录同步失败。
    for _ in 0..<2 {
      if (try? faulty.confirmDurable(pending)) != nil { sent += 1 }
    }
    check(sent == 0)
    failSync = false
    try faulty.confirmDurable(pending)
    check(try faulty.read()?.operation_id == pending.operation_id)
    print("memoryImportSelfCheck=PASS checks=\(checks) synthetic_only=true")
  }
}
