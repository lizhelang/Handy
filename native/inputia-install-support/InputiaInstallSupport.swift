import Darwin
import Foundation
import Security
import CryptoKit

enum InstallCodeError: Error {
  case rejected(String, OSStatus = 0)
}

struct InstallSubject: Codable, Equatable {
  let transaction_id: String
  let plan_sha256: String
  let installation_id: String
  let new_release_id: String
}

struct InstallCodeRequest: Codable, Equatable {
  let schema_version: UInt32
  let subject: InstallSubject
  let purpose: String
  let product_id: String
  let role: String
  let exact_bundle_path: String
  let bundle_id: String
  let release_id: String
  let version: String
  let build: UInt64
  let source_commit: String
  let team_id: String
  let architectures: [String]
  let cdhashes: [String]
}

struct InstallSliceEvidence: Codable, Equatable {
  let architecture: String
  let cdhash: String
  let identifier: String
  let team_id: String
  let hardened_runtime: Bool
  let forbidden_entitlements_absent: Bool
  let flags: UInt32
  let entitlements_sha256: String
}

struct InstallCodeEvidence: Codable, Equatable {
  let schema_version: UInt32
  let request: InstallCodeRequest
  let slices: [InstallSliceEvidence]
  let developer_id_requirement: Bool
  let notarized_requirement: Bool
  let nested_code_integrity_checked: Bool
  let bundle_device: UInt64
  let bundle_inode: UInt64
}

private struct InstallCodeReply: Codable {
  let ok: Bool
  let evidence: InstallCodeEvidence?
  let code: String?
  let os_status: Int32?
}

let installCodeInputLimit = 32_768

func installCanonical<T: Encodable>(_ value: T) throws -> Data {
  let encoder = JSONEncoder()
  encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
  return try encoder.encode(value)
}

private func matches(_ value: String, _ expression: String) -> Bool {
  guard let range = value.range(of: expression, options: .regularExpression) else { return false }
  return range.lowerBound == value.startIndex && range.upperBound == value.endIndex
}

func validateInstallRequest(_ request: InstallCodeRequest) throws {
  let uuid = "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
  let zero = "00000000-0000-0000-0000-000000000000"
  guard request.schema_version == 1, request.product_id == "com.inputia",
    ["control", "ime", "settings", "updater", "bootstrap"].contains(request.role),
    matches(request.subject.transaction_id, uuid), request.subject.transaction_id != zero,
    matches(request.subject.installation_id, uuid), request.subject.installation_id != zero,
    matches(request.subject.plan_sha256, "^[0-9a-f]{64}$"),
    matches(request.subject.new_release_id, "^inputia-[A-Za-z0-9_.-]{1,180}$"),
    matches(request.release_id, "^inputia-[A-Za-z0-9_.-]{1,180}$"),
    (request.purpose == "new_release" && request.release_id == request.subject.new_release_id)
      || (request.purpose == "previous_release" && request.release_id != request.subject.new_release_id),
    matches(request.bundle_id, "^[A-Za-z0-9][A-Za-z0-9.-]{1,190}$"),
    matches(request.team_id, "^[A-Z0-9]{10}$"),
    matches(request.version, "^[0-9]+[.][0-9]+[.][0-9]+$"), request.build > 0,
    matches(request.source_commit, "^(?:[0-9a-f]{40}|[0-9a-f]{64})$"),
    !request.architectures.isEmpty, request.architectures.count <= 3,
    request.architectures == Array(Set(request.architectures)).sorted(),
    request.architectures.allSatisfy({ ["arm64", "arm64e", "x86_64"].contains($0) }),
    request.cdhashes.count == request.architectures.count,
    request.cdhashes == Array(Set(request.cdhashes)).sorted(),
    request.cdhashes.allSatisfy({ matches($0, "^[0-9a-f]{40}$") }) else {
    throw InstallCodeError.rejected("invalid_request")
  }
}

func decodeInstallRequest(_ data: Data) throws -> InstallCodeRequest {
  guard !data.isEmpty, data.count <= installCodeInputLimit else { throw InstallCodeError.rejected("invalid_request") }
  let request: InstallCodeRequest
  do { request = try JSONDecoder().decode(InstallCodeRequest.self, from: data) }
  catch { throw InstallCodeError.rejected("invalid_request") }
  // 固定字段重编码必须逐字节相等：未知键、重复键、浮点整数、非规范空白都不被吞掉。
  guard try installCanonical(request) == data else { throw InstallCodeError.rejected("noncanonical_request") }
  try validateInstallRequest(request)
  return request
}

/// 固定公开 requirement；调用方不能注入任意表达式或选择跳过公证。
func installRequirement(_ request: InstallCodeRequest) throws -> SecRequirement {
  try validateInstallRequest(request)
  let hashes = request.cdhashes.map { "cdhash H\"\($0)\"" }.joined(separator: " or ")
  let expression = "anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists"
    + " and certificate leaf[field.1.2.840.113635.100.6.1.13] exists"
    + " and certificate leaf[subject.OU] = \"\(request.team_id)\""
    + " and identifier \"\(request.bundle_id)\" and (\(hashes)) and notarized"
  var result: SecRequirement?
  let status = SecRequirementCreateWithString(expression as CFString, [], &result)
  guard status == errSecSuccess, let result else { throw InstallCodeError.rejected("requirement_unavailable", status) }
  return result
}

private func safeAncestor(_ metadata: stat) -> Bool {
  metadata.st_mode & S_IFMT == S_IFDIR
    && (metadata.st_uid == geteuid() || metadata.st_uid == 0)
    && (metadata.st_mode & 0o022 == 0 || (metadata.st_uid == 0 && metadata.st_mode & 0o1000 != 0))
}

private func safeBundleNode(_ metadata: stat) -> Bool {
  let type = metadata.st_mode & S_IFMT
  return metadata.st_uid == geteuid() && metadata.st_mode & 0o7022 == 0
    && (type == S_IFDIR || (type == S_IFREG && metadata.st_nlink == 1))
}

/// 对祖先与根逐级 NOFOLLOW；只验证当前用户拥有的受管制品树，不把路径当永久锁。
private final class BundleAnchor {
  let path: String
  let fd: Int32
  let device: UInt64
  let inode: UInt64

  init(_ path: String) throws {
    guard path.utf8.count <= 4096, path.hasPrefix("/"), path.hasSuffix(".app"),
      !path.contains("//"), !path.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains),
      !path.split(separator: "/").contains(where: { $0 == "." || $0 == ".." }) else { throw InstallCodeError.rejected("unsafe_path") }
    var parent = Darwin.open("/", O_RDONLY | O_DIRECTORY | O_CLOEXEC)
    guard parent >= 0 else { throw InstallCodeError.rejected("path_unavailable") }
    do {
      var ancestor = stat()
      guard fstat(parent, &ancestor) == 0, safeAncestor(ancestor) else { throw InstallCodeError.rejected("unsafe_path") }
      for component in path.split(separator: "/") {
        let next = String(component).withCString { Darwin.openat(parent, $0, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC) }
        guard next >= 0 else { throw InstallCodeError.rejected("unsafe_path") }
        Darwin.close(parent); parent = next
        guard fstat(parent, &ancestor) == 0, safeAncestor(ancestor) else { throw InstallCodeError.rejected("unsafe_path") }
      }
      var metadata = stat()
      guard fstat(parent, &metadata) == 0, metadata.st_mode & S_IFMT == S_IFDIR,
        safeBundleNode(metadata) else { throw InstallCodeError.rejected("unsafe_path") }
      self.path = path; fd = parent; device = UInt64(UInt32(bitPattern: metadata.st_dev)); inode = metadata.st_ino
    } catch { Darwin.close(parent); throw error }
  }
  deinit { Darwin.close(fd) }
  func recheck() throws {
    let fresh = try BundleAnchor(path)
    guard fresh.device == device, fresh.inode == inode else { throw InstallCodeError.rejected("path_changed") }
  }
}

/// Security 的 nested check 只覆盖标准代码目录。外层事务仍对整个树做摘要前后核验。
private func validateContainedLinks(_ root: String) throws {
  let url = URL(fileURLWithPath: root, isDirectory: true)
  var failed = false
  guard let items = FileManager.default.enumerator(at: url, includingPropertiesForKeys: [.isSymbolicLinkKey],
    errorHandler: { _, _ in failed = true; return false }) else { throw InstallCodeError.rejected("path_unavailable") }
  var count = 0
  while let item = items.nextObject() as? URL {
    count += 1
    guard count <= 300_000, item.pathComponents.count - url.pathComponents.count <= 128 else {
      throw InstallCodeError.rejected("bundle_budget")
    }
    var metadata = stat()
    guard item.path.withCString({ lstat($0, &metadata) }) == 0 else { throw InstallCodeError.rejected("path_changed") }
    if metadata.st_mode & S_IFMT == S_IFLNK {
      guard metadata.st_uid == geteuid() else { throw InstallCodeError.rejected("unsafe_nested_link") }
      let target = try FileManager.default.destinationOfSymbolicLink(atPath: item.path)
      guard !target.hasPrefix("/"), !target.isEmpty, target.utf8.count <= 4096,
        !target.unicodeScalars.contains(where: CharacterSet.controlCharacters.contains) else { throw InstallCodeError.rejected("unsafe_nested_link") }
      guard let rawResolved = item.path.withCString({ realpath($0, nil) }) else { throw InstallCodeError.rejected("unsafe_nested_link") }
      let resolved = String(cString: rawResolved)
      free(rawResolved)
      guard resolved.hasPrefix(root + "/"), FileManager.default.fileExists(atPath: resolved),
        resolved != item.path else { throw InstallCodeError.rejected("unsafe_nested_link") }
    } else if !safeBundleNode(metadata) {
      throw InstallCodeError.rejected("unsafe_bundle_entry")
    }
  }
  if failed { throw InstallCodeError.rejected("path_unavailable") }
}

private func hex(_ data: Data) -> String { data.map { String(format: "%02x", $0) }.joined() }

/// 只从已通过 Security 验证的 secured Info.plist/代码目录读取字段。
func validateInstallSigningInfo(_ info: [String: Any], request: InstallCodeRequest, architecture: String) throws -> InstallSliceEvidence {
  guard let identifier = info[kSecCodeInfoIdentifier as String] as? String, identifier == request.bundle_id,
    let team = info[kSecCodeInfoTeamIdentifier as String] as? String, team == request.team_id,
    let hash = info[kSecCodeInfoUnique as String] as? Data, hash.count == 20,
    request.cdhashes.contains(hex(hash)) else { throw InstallCodeError.rejected("code_identity_mismatch") }
  guard let flags = info[kSecCodeInfoFlags as String] as? NSNumber,
    flags.uint32Value & 0x10000 != 0, flags.uint32Value & 0x2 == 0 else { throw InstallCodeError.rejected("runtime_required") }
  let forbidden = ["com.apple.security.get-task-allow", "com.apple.security.cs.disable-library-validation",
    "com.apple.security.cs.allow-unsigned-executable-memory", "com.apple.security.cs.allow-dyld-environment-variables",
    "com.apple.security.cs.debugger", "com.apple.private.skip-library-validation"]
  let entitlements: [String: Any]
  if let raw = info[kSecCodeInfoEntitlementsDict as String] {
    guard let decoded = raw as? [String: Any] else { throw InstallCodeError.rejected("invalid_entitlements") }
    entitlements = decoded
  } else {
    guard info[kSecCodeInfoEntitlements as String] == nil else { throw InstallCodeError.rejected("invalid_entitlements") }
    entitlements = [:]
  }
  for key in forbidden {
    if let value = entitlements[key] {
      guard let boolean = value as? NSNumber, CFGetTypeID(boolean) == CFBooleanGetTypeID(),
        !boolean.boolValue else { throw InstallCodeError.rejected("forbidden_entitlement") }
    }
  }
  let allowed: [String: Bool]
  switch request.role {
  case "control": allowed = ["com.apple.security.device.microphone": true, "com.apple.security.device.audio-input": true]
  case "ime": allowed = ["com.apple.security.app-sandbox": false]
  default: allowed = [:]
  }
  var typedEntitlements: [String: Bool] = [:]
  for (key, value) in entitlements {
    guard let required = allowed[key], let boolean = value as? NSNumber,
      CFGetTypeID(boolean) == CFBooleanGetTypeID(), boolean.boolValue == required else {
      throw InstallCodeError.rejected("unexpected_entitlement")
    }
    typedEntitlements[key] = boolean.boolValue
  }
  guard let plist = info[kSecCodeInfoPList as String] as? [String: Any],
    plist["CFBundleIdentifier"] as? String == request.bundle_id,
    plist["CFBundleShortVersionString"] as? String == request.version,
    plist["CFBundleVersion"] as? String == String(request.build),
    plist["InputiaReleaseID"] as? String == request.release_id,
    plist["InputiaSourceCommit"] as? String == request.source_commit else { throw InstallCodeError.rejected("bundle_metadata_mismatch") }
  return .init(architecture: architecture, cdhash: hex(hash), identifier: identifier, team_id: team,
    hardened_runtime: true, forbidden_entitlements_absent: true, flags: flags.uint32Value,
    entitlements_sha256: hex(Data(SHA256.hash(data: try installCanonical(typedEntitlements)))))
}

private func architecture(cpu: UInt32, subtype: UInt32) throws -> String {
  switch (cpu, subtype & 0x00ffffff) {
  case (0x0100000c, 0): return "arm64"
  case (0x0100000c, 2): return "arm64e"
  case (0x01000007, 3): return "x86_64"
  default: throw InstallCodeError.rejected("unsupported_architecture")
  }
}

/// 有界读取 Mach-O 头；不依赖 lipo/xcrun，也不只验证宿主首选 slice。
func inspectMachOArchitectures(_ data: Data, fileSize: UInt64) throws -> [String] {
  func number(_ offset: Int, little: Bool) throws -> UInt32 {
    guard offset >= 0, offset + 4 <= data.count else { throw InstallCodeError.rejected("invalid_macho") }
    let value = data[offset..<(offset + 4)].reduce(UInt32(0)) { ($0 << 8) | UInt32($1) }
    return little ? value.byteSwapped : value
  }
  func wide(_ offset: Int, little: Bool) throws -> UInt64 {
    let first = UInt64(try number(offset, little: little)), second = UInt64(try number(offset + 4, little: little))
    return little ? first | (second << 32) : (first << 32) | second
  }
  let magic = try number(0, little: false)
  if magic == 0xcffaedfe || magic == 0xfeedfacf {
    let little = magic == 0xcffaedfe
    guard fileSize >= 32, data.count >= 32 else { throw InstallCodeError.rejected("invalid_macho") }
    return [try architecture(cpu: number(4, little: little), subtype: number(8, little: little))]
  }
  guard [0xcafebabe, 0xbebafeca, 0xcafebabf, 0xbfbafeca].contains(magic) else { throw InstallCodeError.rejected("invalid_macho") }
  let little = magic == 0xbebafeca || magic == 0xbfbafeca
  let is64 = magic == 0xcafebabf || magic == 0xbfbafeca
  let count = Int(try number(4, little: little)), stride = is64 ? 32 : 20
  guard count > 0, count <= 3, data.count >= 8 + count * stride else { throw InstallCodeError.rejected("invalid_macho") }
  var result: [String] = [], ranges: [(UInt64, UInt64)] = []
  for index in 0..<count {
    let position = 8 + index * stride
    let name = try architecture(cpu: number(position, little: little), subtype: number(position + 4, little: little))
    let offset = try is64 ? wide(position + 8, little: little) : UInt64(number(position + 8, little: little))
    let size = try is64 ? wide(position + 16, little: little) : UInt64(number(position + 12, little: little))
    let alignment = try number(position + (is64 ? 24 : 16), little: little)
    guard offset >= UInt64(8 + count * stride), size >= 32, offset <= fileSize, size <= fileSize - offset,
      alignment < 32, offset % (UInt64(1) << alignment) == 0,
      !ranges.contains(where: { offset < $0.1 && offset + size > $0.0 }), !result.contains(name) else { throw InstallCodeError.rejected("invalid_macho") }
    ranges.append((offset, offset + size)); result.append(name)
  }
  return result.sorted()
}

private func actualArchitectures(_ info: [String: Any], root: String) throws -> [String] {
  guard let executable = info[kSecCodeInfoMainExecutable as String] as? URL,
    let plist = info[kSecCodeInfoPList as String] as? [String: Any],
    let name = plist["CFBundleExecutable"] as? String,
    matches(name, "^[A-Za-z0-9_.-]{1,128}$"), name != ".", name != "..",
    executable.path == root + "/Contents/MacOS/" + name else { throw InstallCodeError.rejected("executable_path_mismatch") }
  var fd = Darwin.open("/", O_RDONLY | O_DIRECTORY | O_CLOEXEC)
  guard fd >= 0 else { throw InstallCodeError.rejected("path_unavailable") }
  defer { Darwin.close(fd) }
  let parts = executable.path.split(separator: "/")
  for (index, component) in parts.enumerated() {
    let next = String(component).withCString { Darwin.openat(fd, $0, O_RDONLY | O_NOFOLLOW | O_CLOEXEC | (index + 1 == parts.count ? O_NONBLOCK : O_DIRECTORY)) }
    guard next >= 0 else { throw InstallCodeError.rejected("unsafe_executable_path") }
    Darwin.close(fd); fd = next
  }
  var before = stat(), after = stat()
  guard fstat(fd, &before) == 0, before.st_mode & S_IFMT == S_IFREG, before.st_nlink == 1,
    before.st_size >= 32 else { throw InstallCodeError.rejected("invalid_macho") }
  var buffer = [UInt8](repeating: 0, count: 4096)
  let count = buffer.withUnsafeMutableBytes { pread(fd, $0.baseAddress!, $0.count, 0) }
  guard count >= 0, fstat(fd, &after) == 0, before.st_ino == after.st_ino,
    before.st_size == after.st_size, before.st_mtimespec.tv_sec == after.st_mtimespec.tv_sec,
    before.st_mtimespec.tv_nsec == after.st_mtimespec.tv_nsec else { throw InstallCodeError.rejected("path_changed") }
  return try inspectMachOArchitectures(Data(buffer.prefix(count)), fileSize: UInt64(before.st_size))
}

// 仅收集已验 SecStaticCode 的安全字典；不重新打开路径，也不把普通 CFBundle 元数据当作签名内容。
struct SecuredInstallPlist {
  private(set) var value: [String: Any]?
  mutating func include(_ info: [String: Any]) throws {
    guard let plist = info[kSecCodeInfoPList as String] as? [String: Any] else {
      throw InstallCodeError.rejected("secured_plist_unavailable")
    }
    if let value, !NSDictionary(dictionary: value).isEqual(to: plist) {
      throw InstallCodeError.rejected("secured_plist_architecture_mismatch")
    }
    value = plist
  }
}

func verifyInstallCode(_ request: InstallCodeRequest,
  captureSecuredPlist: (([String: Any]) throws -> Void)? = nil
) throws -> InstallCodeEvidence {
  try validateInstallRequest(request)
  let root = try BundleAnchor(request.exact_bundle_path)
  try validateContainedLinks(root.path)
  let requirement = try installRequirement(request)
  let flags = SecCSFlags(rawValue: kSecCSCheckAllArchitectures | kSecCSCheckNestedCode | kSecCSStrictValidate)
  var slices: [InstallSliceEvidence] = []
  var securedPlist = SecuredInstallPlist()
  for name in request.architectures {
    var code: SecStaticCode?
    let attributes = [kSecCodeAttributeArchitecture as String: name] as CFDictionary
    let created = SecStaticCodeCreateWithPathAndAttributes(URL(fileURLWithPath: root.path) as CFURL, [], attributes, &code)
    guard created == errSecSuccess, let code else { throw InstallCodeError.rejected("static_code_unavailable", created) }
    let checked = SecStaticCodeCheckValidity(code, flags, requirement)
    guard checked == errSecSuccess else { throw InstallCodeError.rejected("untrusted_signature", checked) }
    var path: CFURL?, raw: CFDictionary?
    guard SecCodeCopyPath(code, [], &path) == errSecSuccess, let path, (path as URL).path == root.path,
      SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &raw) == errSecSuccess,
      let info = raw as? [String: Any] else { throw InstallCodeError.rejected("code_information_unavailable") }
    guard try actualArchitectures(info, root: root.path) == request.architectures else { throw InstallCodeError.rejected("architecture_mismatch") }
    slices.append(try validateInstallSigningInfo(info, request: request, architecture: name))
    let checkedAgain = SecStaticCodeCheckValidity(code, flags, requirement)
    guard checkedAgain == errSecSuccess else { throw InstallCodeError.rejected("code_changed", checkedAgain) }
    if captureSecuredPlist != nil { try securedPlist.include(info) }
  }
  guard slices.map({ $0.cdhash }).sorted() == request.cdhashes else { throw InstallCodeError.rejected("cdhash_set_mismatch") }
  try validateContainedLinks(root.path)
  try root.recheck()
  if let captureSecuredPlist {
    guard let plist = securedPlist.value else { throw InstallCodeError.rejected("secured_plist_unavailable") }
    try captureSecuredPlist(plist)
  }
  return .init(schema_version: 1, request: request, slices: slices,
    developer_id_requirement: true, notarized_requirement: true, nested_code_integrity_checked: true,
    bundle_device: root.device, bundle_inode: root.inode)
}

/// 返回 UTF-8 JSON；没有生产测试模式或宽松回退，错误不回显任意路径/证书内容。
@_cdecl("iuis_verify_code")
public func iuisVerifyCode(_ bytes: UnsafePointer<UInt8>?, _ length: UInt) -> UnsafeMutablePointer<CChar>? {
  let reply: InstallCodeReply
  do {
    guard let bytes, length > 0, length <= UInt(installCodeInputLimit) else { throw InstallCodeError.rejected("invalid_request") }
    let request = try decodeInstallRequest(Data(bytes: bytes, count: Int(length)))
    reply = .init(ok: true, evidence: try verifyInstallCode(request), code: nil, os_status: nil)
  } catch InstallCodeError.rejected(let code, let status) {
    reply = .init(ok: false, evidence: nil, code: code, os_status: status)
  } catch {
    reply = .init(ok: false, evidence: nil, code: "verification_unavailable", os_status: 0)
  }
  guard let data = try? installCanonical(reply), let json = String(data: data, encoding: .utf8) else { return nil }
  return strdup(json)
}

@_cdecl("iuis_string_free")
public func iuisStringFree(_ string: UnsafeMutablePointer<CChar>?) { free(string) }
