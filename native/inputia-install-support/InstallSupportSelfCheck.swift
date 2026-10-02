import Darwin
import Foundation
import Security

private var assertions = 0
private func check(_ value: Bool, _ label: String) {
  precondition(value, label); assertions += 1
}
private func rejects(_ label: String, code: String? = nil, _ operation: () throws -> Void) {
  do { try operation(); preconditionFailure("unexpected acceptance: \(label)") }
  catch InstallCodeError.rejected(let reason, _) {
    if let code { check(reason == code, label + ": " + reason) }
    else { assertions += 1 }
  } catch { preconditionFailure("untyped error: \(label): \(error)") }
}
private func request(_ path: String, hash: String = String(repeating: "11", count: 20), role: String = "control") -> InstallCodeRequest {
  .init(schema_version: 1,
    subject: .init(transaction_id: "11111111-1111-4111-8111-111111111111", plan_sha256: String(repeating: "a", count: 64),
      installation_id: "22222222-2222-4222-8222-222222222222", new_release_id: "inputia-1.1.0-test"),
    purpose: "new_release", product_id: "com.inputia", role: role, exact_bundle_path: path, bundle_id: "com.inputia.test.native",
    release_id: "inputia-1.1.0-test", version: "1.1.0", build: 84, source_commit: String(repeating: "b", count: 40),
    team_id: "TESTTEAM01", architectures: ["arm64"], cdhashes: [hash])
}
private func replacement(_ value: InstallCodeRequest, _ key: String, _ data: Any) throws -> Data {
  var object = try JSONSerialization.jsonObject(with: installCanonical(value)) as! [String: Any]
  object[key] = data
  return try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys, .withoutEscapingSlashes])
}
private func command(_ executable: String, _ arguments: [String]) throws {
  let process = Process(), pipe = Pipe()
  process.executableURL = URL(fileURLWithPath: executable); process.arguments = arguments
  process.standardOutput = pipe; process.standardError = pipe
  try process.run()
  let output = pipe.fileHandleForReading.readDataToEndOfFile()
  process.waitUntilExit()
  guard process.terminationStatus == 0 else { throw NSError(domain: "synthetic-fixture", code: Int(process.terminationStatus),
    userInfo: [NSLocalizedDescriptionKey:String(data: output, encoding: .utf8) ?? "command failed"]) }
}
private func info(_ request: InstallCodeRequest) -> [String: Any] {
  [kSecCodeInfoIdentifier as String: request.bundle_id,
    kSecCodeInfoTeamIdentifier as String: request.team_id,
    kSecCodeInfoUnique as String: Data(repeating: 0x11, count: 20),
    kSecCodeInfoFlags as String: NSNumber(value: 0x10000),
    kSecCodeInfoEntitlementsDict as String: ["com.apple.security.device.microphone":true, "com.apple.security.device.audio-input":true],
    kSecCodeInfoPList as String: ["CFBundleIdentifier":request.bundle_id,"CFBundleShortVersionString":request.version,
      "CFBundleVersion":String(request.build),"InputiaReleaseID":request.release_id,"InputiaSourceCommit":request.source_commit]]
}

@main
struct InstallSupportSelfCheck {
  static func main() throws {
    let physical = realpath(FileManager.default.temporaryDirectory.path, nil)!
    let temporary = URL(fileURLWithPath: String(cString: physical)).appendingPathComponent("inputia-native-verify-" + UUID().uuidString)
    free(physical)
    try FileManager.default.createDirectory(at: temporary, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: temporary) }
    let root = temporary.appendingPathComponent("合成.app"), macos = root.appendingPathComponent("Contents/MacOS")
    try FileManager.default.createDirectory(at: macos, withIntermediateDirectories: true)
    let expected = request(root.path)
    let raw = try installCanonical(expected)
    check(try decodeInstallRequest(raw) == expected, "canonical decode including unicode path")
    let sha256Commit = try decodeInstallRequest(replacement(expected, "source_commit", String(repeating: "c", count: 64)))
    check(sha256Commit.source_commit.count == 64, "sha256 source commit accepted")
    rejects("invalid source commit length") {
      _ = try decodeInstallRequest(replacement(expected, "source_commit", String(repeating: "c", count: 63)))
    }
    rejects("uppercase source commit") {
      _ = try decodeInstallRequest(replacement(expected, "source_commit", String(repeating: "A", count: 64)))
    }
    _ = try installRequirement(expected)
    assertions += 1
    for (key, value) in [("skip_notarization", true as Any), ("schema_version", 2), ("role", "other"),
      ("product_id", "other"), ("team_id", "A\" or true"), ("release_id", "inputia-other"),
      ("architectures", ["arm64", "arm64"]), ("cdhashes", [String(repeating: "A", count: 40)])] {
      rejects("request \(key)") { _ = try decodeInstallRequest(replacement(expected, key, value)) }
    }
    rejects("duplicate key") {
      _ = try decodeInstallRequest(Data(("{\"build\":84," + String(decoding: raw.dropFirst(), as: UTF8.self)).utf8))
    }
    rejects("oversized request") { _ = try decodeInstallRequest(Data(repeating: 0x20, count: installCodeInputLimit + 1)) }
    rejects("trailing whitespace") { _ = try decodeInstallRequest(raw + Data([0x20])) }
    rejects("previous release cannot equal new target") { _ = try decodeInstallRequest(replacement(expected, "purpose", "previous_release")) }
    var oldObject = try JSONSerialization.jsonObject(with: raw) as! [String: Any]
    oldObject["purpose"] = "previous_release"; oldObject["release_id"] = "inputia-1.0.0-previous"
    let oldRequest = try decodeInstallRequest(JSONSerialization.data(withJSONObject: oldObject, options: [.sortedKeys, .withoutEscapingSlashes]))
    check(oldRequest.subject == expected.subject && oldRequest.release_id != expected.release_id, "previous identity retains transaction target")
    for (pointer, length): (UnsafePointer<UInt8>?, UInt) in [(nil, 1), (nil, 0)] {
      let reply = iuisVerifyCode(pointer, length)!
      check(String(cString: reply).contains("invalid_request"), "ABI null rejected"); iuisStringFree(reply)
    }
    var single: UInt8 = 0
    let reply = withUnsafePointer(to: &single) { iuisVerifyCode($0, UInt.max)! }
    check(String(cString: reply).contains("invalid_request"), "ABI size checked before reading"); iuisStringFree(reply)

    // 以下为纯策略单测；合成 signing info 绝不经生产验证返回 VerifiedCodeEvidence。
    let observed = info(expected)
    let policy = try validateInstallSigningInfo(observed, request: expected, architecture: "arm64")
    check(policy.entitlements_sha256.count == 64 && policy.flags == 0x10000, "synthetic policy evidence")
    for (key, value, code) in [(kSecCodeInfoIdentifier as String, "other" as Any, "code_identity_mismatch"),
      (kSecCodeInfoTeamIdentifier as String,"OTHERTEAM1","code_identity_mismatch"),
      (kSecCodeInfoUnique as String,Data(repeating: 0x22, count: 20),"code_identity_mismatch"),
      (kSecCodeInfoFlags as String,NSNumber(value: 0),"runtime_required"),
      (kSecCodeInfoFlags as String,NSNumber(value: 0x10002),"runtime_required")] {
      var modified = observed; modified[key] = value
      rejects("signing info \(key)", code: code) { _ = try validateInstallSigningInfo(modified, request: expected, architecture: "arm64") }
    }
    for bad: Any in [true, "false", 0] {
      var modified = observed; modified[kSecCodeInfoEntitlementsDict as String] = ["com.apple.security.cs.disable-library-validation":bad]
      rejects("forbidden entitlement type", code: "forbidden_entitlement") { _ = try validateInstallSigningInfo(modified, request: expected, architecture: "arm64") }
    }
    var modified = observed
    modified[kSecCodeInfoEntitlementsDict as String] = ["unknown.entitlement":false]
    rejects("unknown entitlement", code: "unexpected_entitlement") { _ = try validateInstallSigningInfo(modified, request: expected, architecture: "arm64") }
    modified = observed; modified.removeValue(forKey: kSecCodeInfoEntitlementsDict as String)
    modified[kSecCodeInfoEntitlements as String] = Data([1,2,3])
    rejects("unparsed raw entitlement", code: "invalid_entitlements") { _ = try validateInstallSigningInfo(modified, request: expected, architecture: "arm64") }
    modified = observed
    modified[kSecCodeInfoPList as String] = ["CFBundleIdentifier":expected.bundle_id,"InputiaReleaseID":"inputia-wrong"]
    rejects("wrong sealed release", code: "bundle_metadata_mismatch") { _ = try validateInstallSigningInfo(modified, request: expected, architecture: "arm64") }

    func words(_ values: [UInt32]) -> Data { Data(values.flatMap { v in [UInt8(v >> 24), UInt8(truncatingIfNeeded: v >> 16), UInt8(truncatingIfNeeded: v >> 8), UInt8(truncatingIfNeeded: v)] }) }
    let thin = words([0xcffaedfe, 0x0c000001, 0, 0, 0, 0, 0, 0])
    check(try inspectMachOArchitectures(thin, fileSize: 32) == ["arm64"], "thin architecture")
    let fat = words([0xcafebabe, 2, 0x0100000c, 0, 4096, 64, 12, 0x01000007, 3, 8192, 64, 12])
    check(try inspectMachOArchitectures(fat, fileSize: 8256) == ["arm64", "x86_64"], "all fat slices enumerated")
    rejects("fat outside bounds") { _ = try inspectMachOArchitectures(fat, fileSize: 8192) }
    rejects("unknown binary") { _ = try inspectMachOArchitectures(Data(repeating: 0, count: 32), fileSize: 32) }

    // 真正调用系统 Security，但只有临时 ad-hoc 负例，没有 Developer ID 成功声明。
    let source = temporary.appendingPathComponent("fixture.c")
    try "int main(void) { return 0; }\n".write(to: source, atomically: true, encoding: .utf8)
    try command("/usr/bin/clang", ["-target","arm64-apple-macos13.0", source.path,"-o",macos.appendingPathComponent("Fixture").path])
    let plist: [String:Any] = ["CFBundleIdentifier":expected.bundle_id,"CFBundleExecutable":"Fixture","CFBundlePackageType":"APPL",
      "CFBundleVersion":"84","CFBundleShortVersionString":"1.1.0","InputiaReleaseID":expected.release_id,"InputiaSourceCommit":expected.source_commit]
    try PropertyListSerialization.data(fromPropertyList: plist, format: .xml, options: 0).write(to: root.appendingPathComponent("Contents/Info.plist"))
    try command("/usr/bin/codesign", ["--force","--sign","-","--options","runtime",root.path])
    var code: SecStaticCode?, rawInfo: CFDictionary?
    check(SecStaticCodeCreateWithPath(root as CFURL, [], &code) == errSecSuccess, "fixture static code")
    check(SecCodeCopySigningInformation(code!, SecCSFlags(rawValue: kSecCSSigningInformation), &rawInfo) == errSecSuccess, "fixture hash read")
    let hash = (rawInfo as! [String:Any])[kSecCodeInfoUnique as String] as! Data
    let actualHash = hash.map { String(format: "%02x", $0) }.joined()
    let adhoc = request(root.path, hash: actualHash)
    rejects("real ad-hoc never becomes trusted", code: "untrusted_signature") { _ = try verifyInstallCode(adhoc) }
    try FileManager.default.setAttributes([.posixPermissions:0o775], ofItemAtPath: temporary.path)
    rejects("writable ancestor", code: "unsafe_path") { _ = try verifyInstallCode(adhoc) }
    try FileManager.default.setAttributes([.posixPermissions:0o700], ofItemAtPath: temporary.path)
    try FileManager.default.setAttributes([.posixPermissions:0o775], ofItemAtPath: macos.path)
    rejects("writable internal directory", code: "unsafe_bundle_entry") { _ = try verifyInstallCode(adhoc) }
    try FileManager.default.setAttributes([.posixPermissions:0o755], ofItemAtPath: macos.path)
    try FileManager.default.setAttributes([.posixPermissions:0o664], ofItemAtPath: root.appendingPathComponent("Contents/Info.plist").path)
    rejects("writable internal file", code: "unsafe_bundle_entry") { _ = try verifyInstallCode(adhoc) }
    try FileManager.default.setAttributes([.posixPermissions:0o644], ofItemAtPath: root.appendingPathComponent("Contents/Info.plist").path)
    let symlink = temporary.appendingPathComponent("alias.app")
    try FileManager.default.createSymbolicLink(at: symlink, withDestinationURL: root)
    rejects("root symlink", code: "unsafe_path") { _ = try verifyInstallCode(request(symlink.path, hash: actualHash)) }
    let inner = root.appendingPathComponent("Contents/escape")
    try FileManager.default.createSymbolicLink(atPath: inner.path, withDestinationPath: "../../outside")
    rejects("outside symlink", code: "unsafe_nested_link") { _ = try verifyInstallCode(adhoc) }
    try FileManager.default.removeItem(at: inner)
    let resource = root.appendingPathComponent("Contents/Frameworks/Fixture.framework/Versions/A/Resources")
    try FileManager.default.createDirectory(at: resource, withIntermediateDirectories: true)
    try Data([1]).write(to: resource.appendingPathComponent("fixture"))
    try FileManager.default.createSymbolicLink(atPath: resource.deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Current").path, withDestinationPath: "A")
    try FileManager.default.createSymbolicLink(atPath: resource.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Resources").path, withDestinationPath: "Versions/Current/Resources")
    rejects("legal Framework links reach Security, still ad-hoc", code: "untrusted_signature") { _ = try verifyInstallCode(adhoc) }
    print("native_install_security_checks=\(assertions) real_developer_id_positive=NOT_RUN system_install_touched=false")
  }
}
