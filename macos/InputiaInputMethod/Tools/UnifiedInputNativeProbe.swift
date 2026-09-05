import AppKit
import ApplicationServices
import Carbon
import Darwin
import Foundation
import InputMethodKit
import Security

// 仅查询能力和权限，不请求权限、不读取输入文字、不注册输入源。
// IPC 演练只连接本工具创建的临时端点和子进程。
private struct ProbeError: Error, CustomStringConvertible {
  let description: String
}

private func checked(_ result: Int32, _ operation: String) throws {
  if result < 0 { throw ProbeError(description: "\(operation): errno=\(errno)") }
}

private func withAddress<T>(_ path: String, _ body: (UnsafePointer<sockaddr>, socklen_t) throws -> T) throws -> T {
  var address = sockaddr_un()
  address.sun_family = sa_family_t(AF_UNIX)
  let bytes = Array(path.utf8CString)
  guard bytes.count <= MemoryLayout.size(ofValue: address.sun_path) else {
    throw ProbeError(description: "socket path exceeds sockaddr_un")
  }
  address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
  withUnsafeMutableBytes(of: &address.sun_path) { destination in
    bytes.withUnsafeBytes { source in destination.copyBytes(from: source) }
  }
  return try withUnsafePointer(to: &address) {
    try $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
      try body($0, socklen_t(MemoryLayout<sockaddr_un>.size))
    }
  }
}

private func client(path: String) throws {
  let descriptor = socket(AF_UNIX, SOCK_STREAM, 0)
  try checked(descriptor, "socket")
  defer { close(descriptor) }
  try withAddress(path) { try checked(connect(descriptor, $0, $1), "connect") }
  var item = pollfd(fd: descriptor, events: Int16(POLLIN), revents: 0)
  guard poll(&item, 1, 5000) > 0 else { throw ProbeError(description: "peer completion timed out") }
}

private func signingEvidence(audit: Data) -> [String: Any] {
  var code: SecCode?
  let copyStatus = SecCodeCopyGuestWithAttributes(
    nil, [kSecGuestAttributeAudit as String: audit] as CFDictionary,
    SecCSFlags(rawValue: 0), &code
  )
  var evidence: [String: Any] = ["audit_lookup_status": copyStatus]
  guard let code, copyStatus == errSecSuccess else { return evidence }
  evidence["validity_status"] = SecCodeCheckValidity(code, SecCSFlags(rawValue: 0), nil)
  var staticCode: SecStaticCode?
  let staticStatus = SecCodeCopyStaticCode(code, SecCSFlags(rawValue: 0), &staticCode)
  evidence["static_code_status"] = staticStatus
  guard let staticCode, staticStatus == errSecSuccess else { return evidence }
  var info: CFDictionary?
  let infoStatus = SecCodeCopySigningInformation(staticCode, SecCSFlags(rawValue: kSecCSSigningInformation), &info)
  evidence["signing_information_status"] = infoStatus
  if let info = info as? [String: Any] {
    evidence["identifier"] = info[kSecCodeInfoIdentifier as String] ?? NSNull()
    evidence["team_identifier"] = info[kSecCodeInfoTeamIdentifier as String] ?? NSNull()
    evidence["signing_flags"] = info[kSecCodeInfoFlags as String] ?? NSNull()
  }
  var requirement: SecRequirement?
  let requirementStatus = SecRequirementCreateWithString(
    "anchor apple generic" as CFString, SecCSFlags(rawValue: 0), &requirement
  )
  evidence["release_requirement_parse_status"] = requirementStatus
  if requirementStatus == errSecSuccess, let requirement {
    evidence["apple_anchor_requirement_status"] = SecCodeCheckValidity(code, SecCSFlags(rawValue: 0), requirement)
  }
  return evidence
}

private func peerEvidence() throws -> [String: Any] {
  // Darwin sockaddr_un 仅容纳 104 字节，系统 TMPDIR 可能很长。
  let root = URL(fileURLWithPath: "/tmp", isDirectory: true).appendingPathComponent("uip-\(UUID().uuidString)")
  try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
  defer { try? FileManager.default.removeItem(at: root) }
  let path = root.appendingPathComponent("peer.sock").path
  let listener = socket(AF_UNIX, SOCK_STREAM, 0)
  try checked(listener, "socket")
  defer { close(listener) }
  try withAddress(path) { try checked(bind(listener, $0, $1), "bind") }
  try checked(chmod(path, 0o600), "chmod socket")
  try checked(listen(listener, 1), "listen")
  let child = Process()
  child.executableURL = URL(fileURLWithPath: CommandLine.arguments[0]).standardizedFileURL
  child.arguments = ["--peer-client", path]
  try child.run()
  defer { if child.isRunning { child.terminate() }; child.waitUntilExit() }
  var item = pollfd(fd: listener, events: Int16(POLLIN), revents: 0)
  guard poll(&item, 1, 5000) > 0 else { throw ProbeError(description: "accept timed out") }
  let peer = accept(listener, nil, nil)
  try checked(peer, "accept")
  defer { close(peer) }
  var uid: uid_t = 0
  var gid: gid_t = 0
  let credentialStatus = getpeereid(peer, &uid, &gid)
  var peerPID: pid_t = 0
  var pidLength = socklen_t(MemoryLayout<pid_t>.size)
  let pidStatus = getsockopt(peer, 0, LOCAL_PEERPID, &peerPID, &pidLength)
  var token = audit_token_t()
  var tokenLength = socklen_t(MemoryLayout<audit_token_t>.size)
  let tokenStatus = getsockopt(peer, 0, LOCAL_PEERTOKEN, &token, &tokenLength)
  let tokenErrno = tokenStatus == 0 ? 0 : errno
  var evidence: [String: Any] = [
    "credential_status": credentialStatus,
    "same_user": credentialStatus == 0 && uid == geteuid(),
    "pid_query_status": pidStatus,
    "matches_spawned_child": pidStatus == 0 && peerPID == child.processIdentifier,
    "audit_token_status": tokenStatus,
    "audit_token_errno": tokenErrno,
    "audit_token_bytes": tokenLength,
    "private_directory_mode": "0700", "socket_mode": "0600",
  ]
  if tokenStatus == 0, tokenLength == MemoryLayout<audit_token_t>.size {
    let data = withUnsafeBytes(of: &token) { Data($0) }
    evidence["peer_code_signature"] = signingEvidence(audit: data)
  }
  var marker: UInt8 = 1
  _ = write(peer, &marker, 1)
  child.waitUntilExit()
  evidence["child_exit_status"] = child.terminationStatus
  return evidence
}

@main
struct UnifiedInputNativeProbe {
  static func main() {
    do {
      if CommandLine.arguments.count == 3, CommandLine.arguments[1] == "--peer-client" {
        try client(path: CommandLine.arguments[2])
        return
      }
      let arguments = CommandLine.arguments
      var rounds = 1
      if arguments.count == 3, arguments[1] == "--rounds", let value = Int(arguments[2]), (1...100).contains(value) {
        rounds = value
      } else if arguments.count != 1 {
        throw ProbeError(description: "usage: UnifiedInputNativeProbe [--rounds 1...100]")
      }
      var peerResults: [[String: Any]] = []
      let started = ProcessInfo.processInfo.systemUptime
      for _ in 0..<rounds { peerResults.append(try peerEvidence()) }
      let source = TISCopyCurrentKeyboardInputSource().takeRetainedValue()
      var inputSourceID = "unknown"
      if let property = TISGetInputSourceProperty(source, kTISPropertyInputSourceID) {
        inputSourceID = Unmanaged<CFString>.fromOpaque(property).takeUnretainedValue() as String
      }
      let result: [String: Any] = [
        "probe_version": 1,
        "timestamp": ISO8601DateFormatter().string(from: Date()),
        "os": ProcessInfo.processInfo.operatingSystemVersionString,
        "processor_count": ProcessInfo.processInfo.processorCount,
        "physical_memory_bytes": ProcessInfo.processInfo.physicalMemory,
        "accessibility_trusted_for_probe": AXIsProcessTrusted(),
        "input_monitoring_preflight_for_probe": CGPreflightListenEventAccess(),
        "event_posting_preflight_for_probe": CGPreflightPostEventAccess(),
        "secure_event_input_enabled": IsSecureEventInputEnabled(),
        "current_input_source_id": inputSourceID,
        "peer_runtime": peerResults,
        "peer_rounds": rounds,
        "peer_elapsed_seconds": ProcessInfo.processInfo.systemUptime - started,
        "imk_framework_linked": true,
        "imk_live_target_validated": false,
        "clipboard_source_coverage_validated": false,
        "scope": "metadata only; no text reads, permission prompts, event injection or input source mutation",
      ]
      let data = try JSONSerialization.data(withJSONObject: result, options: [.prettyPrinted, .sortedKeys])
      print(String(decoding: data, as: UTF8.self))
    } catch {
      fputs("UnifiedInputNativeProbe failed: \(error)\n", stderr)
      exit(1)
    }
  }
}
