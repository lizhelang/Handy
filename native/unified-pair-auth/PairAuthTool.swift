import Darwin
import Foundation
import Security

private func fail(_ reason: String) throws -> Never { throw PairAuthError.invalid(reason) }
private func encode<T: Encodable>(_ value: T) throws -> Data {
  let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
  return try encoder.encode(value)
}
private func read(_ path: String, limit: Int = 16_384, privateKey: Bool = false) throws -> Data {
  let fd = open(path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
  guard fd >= 0 else { try fail("file open") }; defer { close(fd) }
  var metadata = stat()
  guard fstat(fd, &metadata) == 0, metadata.st_mode & S_IFMT == S_IFREG,
        metadata.st_size >= 0, metadata.st_size <= limit else { try fail("file metadata") }
  if privateKey && (metadata.st_uid != geteuid() || metadata.st_mode & 0o077 != 0 || metadata.st_nlink != 1) {
    try fail("private key owner, permissions or hardlink")
  }
  var bytes = [UInt8](repeating: 0, count: limit + 1)
  var total = 0
  while total < bytes.count {
    let count = bytes.withUnsafeMutableBytes { raw in
      Darwin.read(fd, raw.baseAddress!.advanced(by: total), raw.count - total)
    }
    if count < 0 { if errno == EINTR { continue }; try fail("file read") }
    if count == 0 { break }; total += count
  }
  guard total <= limit else { try fail("file grew beyond limit") }
  return Data(bytes.prefix(total))
}
private func writeNew(_ data: Data, _ path: String) throws {
  let fd = open(path, O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0o600)
  guard fd >= 0 else { try fail("exclusive output creation") }; defer { close(fd) }
  try data.withUnsafeBytes { raw in
    var offset = 0
    while offset < raw.count {
      let count = Darwin.write(fd, raw.baseAddress!.advanced(by: offset), raw.count - offset)
      if count < 0 { if errno == EINTR { continue }; try fail("output write") }
      guard count > 0 else { try fail("short output write") }; offset += count
    }
  }
  guard fsync(fd) == 0 else { try fail("output sync") }
}
private func identity(_ path: String, role: PairRole) throws -> PairCodeIdentity {
  var code: SecStaticCode?
  guard SecStaticCodeCreateWithPath(URL(fileURLWithPath: path) as CFURL, [], &code) == errSecSuccess,
        let code, SecStaticCodeCheckValidity(code, [], nil) == errSecSuccess else {
    try fail("build artifact signature")
  }
  var info: CFDictionary?
  guard SecCodeCopySigningInformation(code, SecCSFlags(rawValue: kSecCSSigningInformation), &info) == errSecSuccess,
        let info = info as? [String: Any],
        let identifier = info[kSecCodeInfoIdentifier as String] as? String,
        let hash = info[kSecCodeInfoUnique as String] as? Data else { try fail("build identity") }
  return PairCodeIdentity(role: role, identifier: identifier,
    cdhashes: [hash.map { String(format: "%02x", $0) }.joined()])
}
private func withAddress<T>(_ path: String, _ body: (UnsafePointer<sockaddr>, socklen_t) throws -> T) throws -> T {
  var address = sockaddr_un(); address.sun_family = sa_family_t(AF_UNIX)
  address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
  let bytes = Array(path.utf8CString)
  guard bytes.count <= MemoryLayout.size(ofValue: address.sun_path) else { try fail("socket path") }
  withUnsafeMutableBytes(of: &address.sun_path) { destination in
    bytes.withUnsafeBytes { destination.copyBytes(from: $0) }
  }
  return try withUnsafePointer(to: &address) {
    try $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
      try body($0, socklen_t(MemoryLayout<sockaddr_un>.size))
    }
  }
}
private func listener(_ path: String) throws -> Int32 {
  let fd = socket(AF_UNIX, SOCK_STREAM, 0)
  guard fd >= 0 else { try fail("socket") }
  do {
    try withAddress(path) { guard bind(fd, $0, $1) == 0 else { try fail("bind") } }
    guard chmod(path, 0o600) == 0, listen(fd, 3) == 0 else { try fail("listen") }
    return fd
  } catch { close(fd); throw error }
}
private func ready(_ fd: Int32) throws {
  var event = pollfd(fd: fd, events: Int16(POLLIN), revents: 0)
  guard poll(&event, 1, 5000) > 0 else { try fail("socket wait") }
}
private func fixtureTrust(_ publicKey: Data, role: PairRole, hardened: Bool = false) -> PairTrust {
  PairTrust(publicKeyX963: publicKey, keyID: "synthetic-build-key", runID: "trial-20260905",
    profileID: "unified-candidate:trial-20260905", protocolMajor: 1, localRole: role,
    requireHardenedRuntime: hardened)
}

private func child(_ executable: String, _ arguments: [String]) throws -> Process {
  let process = Process(); process.executableURL = URL(fileURLWithPath: executable)
  process.arguments = arguments; process.standardOutput = FileHandle.nullDevice
  process.standardError = FileHandle.nullDevice
  try process.run(); return process
}
private func stop(_ process: Process) {
  if process.isRunning { process.terminate() }; process.waitUntilExit()
}

// 独立实验入口显式从测试文件读公钥；产品不得使用此引导方式建立信任。
private func fixtureClient(_ args: [String]) throws -> Int32 {
  let fd = socket(AF_UNIX, SOCK_STREAM, 0)
  guard fd >= 0 else { try fail("client socket") }; defer { close(fd) }
  try withAddress(args[0]) { guard connect(fd, $0, $1) == 0 else { try fail("connect") } }
  let manifest = try SignedPairManifest.verify(read(args[1]), trust: fixtureTrust(read(args[2]), role: .inputia))
  var accepted: UInt8 = 0
  do { _ = try PeerAuthenticator.authenticate(socketFD: fd, manifest: manifest, expectedRole: .handy); accepted = 1 }
  catch { accepted = 0 }
  _ = Darwin.write(fd, &accepted, 1)
  try ready(fd)
  var peerAccepted: UInt8 = 0
  guard Darwin.read(fd, &peerAccepted, 1) == 1 else { try fail("server verdict") }
  return accepted == 1 && peerAccepted == 1 ? 0 : 3
}

private func fixtureServer(_ args: [String]) throws -> Int32 {
  let server = try listener(args[0]); defer { close(server); unlink(args[0]) }
  var marker: UInt8 = 1
  _ = Darwin.write(STDOUT_FILENO, &marker, 1)
  try ready(server); let fd = accept(server, nil, nil)
  guard fd >= 0 else { try fail("fixture server accept") }; defer { close(fd) }
  let manifest = try SignedPairManifest.verify(read(args[1]), trust: fixtureTrust(read(args[2]), role: .handy))
  var accepted: UInt8 = 0
  do { _ = try PeerAuthenticator.authenticate(socketFD: fd, manifest: manifest, expectedRole: .inputia); accepted = 1 }
  catch { accepted = 0 }
  try ready(fd); var peerAccepted: UInt8 = 0
  guard Darwin.read(fd, &peerAccepted, 1) == 1 else { try fail("fixture client verdict") }
  _ = Darwin.write(fd, &accepted, 1)
  return accepted == 1 && peerAccepted == 1 ? 0 : 3
}

private func selfCheck(host: String, rogue: String) throws {
  let root = "/tmp/pair-auth-\(UUID().uuidString)"
  guard mkdir(root, 0o700) == 0 else { try fail("fixture directory") }
  defer { try? FileManager.default.removeItem(atPath: root) }
  let ownPath = URL(fileURLWithPath: CommandLine.arguments[0]).standardizedFileURL.path
  let key = try PairBuildKey()
  let manifestPayload = try PairManifestPayload(keyID: "synthetic-build-key", runID: "trial-20260905",
    profileID: "unified-candidate:trial-20260905", protocolMajor: 1,
    peers: [identity(ownPath, role: .handy), identity(host, role: .inputia)])
  let signed = try key.sign(manifestPayload)
  let pubPath = root + "/public.x963"; let manifestPath = root + "/pair.json"
  try writeNew(key.publicKeyX963, pubPath); try writeNew(signed, manifestPath)
  let trust = fixtureTrust(key.publicKeyX963, role: .handy)
  let manifest = try SignedPairManifest.verify(signed, trust: trust)
  var checks = 0
  func reject(_ name: String, _ work: () throws -> Void) throws {
    do { try work() } catch { checks += 1; return }; try fail("negative test accepted: \(name)")
  }
  func require(_ condition: Bool, _ name: String) throws {
    guard condition else { try fail("self check: \(name)") }; checks += 1
  }
  try require(manifest.payload == manifestPayload, "valid signature")
  let imported = try PairBuildKey(privateRepresentationForBuildOnly: key.privateRepresentationForBuildOnly())
  try require(imported.publicKeyX963 == key.publicKeyX963, "ephemeral private key roundtrip")
  try reject("wrong public key") {
    _ = try SignedPairManifest.verify(signed, trust: fixtureTrust(PairBuildKey().publicKeyX963, role: .handy))
  }
  try reject("wrong profile") {
    _ = try SignedPairManifest.verify(signed, trust: PairTrust(publicKeyX963: key.publicKeyX963,
      keyID: trust.keyID, runID: trust.runID, profileID: "other", protocolMajor: 1, localRole: .handy))
  }
  try reject("wrong run") {
    _ = try SignedPairManifest.verify(signed, trust: PairTrust(publicKeyX963: key.publicKeyX963,
      keyID: trust.keyID, runID: "other", profileID: trust.profileID, protocolMajor: 1, localRole: .handy))
  }
  try reject("wrong major") {
    _ = try SignedPairManifest.verify(signed, trust: PairTrust(publicKeyX963: key.publicKeyX963,
      keyID: trust.keyID, runID: trust.runID, profileID: trust.profileID, protocolMajor: 2, localRole: .handy))
  }
  try reject("oversize") { _ = try SignedPairManifest.verify(Data(repeating: 32, count: 16_385), trust: trust) }
  try reject("duplicate envelope field") {
    var object = try JSONSerialization.jsonObject(with: signed) as! [String: String]
    let raw = object.removeValue(forKey: "payloadBase64")!
    let duplicate = "{\"payloadBase64\":\"\(raw)\",\"payloadBase64\":\"\(raw)\",\"signatureBase64\":\"\(object["signatureBase64"]!)\"}"
    _ = try SignedPairManifest.verify(Data(duplicate.utf8), trust: trust)
  }
  // 使用本次内存私钥签名异常原始 payload，确认不是仅靠坏签名掩盖解析缺陷。
  let privateData = try key.privateRepresentationForBuildOnly()
  let attributes: [String: Any] = [kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
    kSecAttrKeyClass as String: kSecAttrKeyClassPrivate, kSecAttrKeySizeInBits as String: 256]
  var error: Unmanaged<CFError>?
  guard let signingKey = SecKeyCreateWithData(privateData as CFData, attributes as CFDictionary, &error) else {
    try fail("fixture signing key")
  }
  func signRaw(_ raw: Data, domain: String = "Handy-Inputia-Candidate-Pair-Manifest-v1\0") throws -> Data {
    guard let signature = SecKeyCreateSignature(signingKey, .ecdsaSignatureMessageX962SHA256,
      (Data(domain.utf8) + raw) as CFData, &error) else { try fail("fixture signature") }
    return try JSONSerialization.data(withJSONObject: ["payloadBase64": raw.base64EncodedString(),
      "signatureBase64": (signature as Data).base64EncodedString()], options: [.sortedKeys, .withoutEscapingSlashes])
  }
  let rawPayload = try encode(manifestPayload)
  try require(manifest.payload.profileID == "unified-candidate:trial-20260905",
    "actual candidate derived profile contract")
  for runID in ["a", String(repeating: "A", count: 64), "trial_20260905-1"] {
    let payload = PairManifestPayload(keyID: trust.keyID, runID: runID,
      profileID: "unified-candidate:\(runID)", protocolMajor: 1, peers: manifestPayload.peers)
    let matchingTrust = PairTrust(publicKeyX963: key.publicKeyX963, keyID: trust.keyID,
      runID: runID, profileID: payload.profileID, protocolMajor: 1, localRole: .handy)
    try require(try SignedPairManifest.verify(key.sign(payload), trust: matchingTrust).payload == payload,
      "valid candidate run boundary")
  }
  for runID in ["", ".", "..", "trial.20260905", "trial:20260905", "trial/20260905",
                "trial 20260905", "试验", String(repeating: "a", count: 65)] {
    let payload = PairManifestPayload(keyID: trust.keyID, runID: runID,
      profileID: "unified-candidate:\(runID)", protocolMajor: 1, peers: manifestPayload.peers)
    let matchingTrust = PairTrust(publicKeyX963: key.publicKeyX963, keyID: trust.keyID,
      runID: runID, profileID: payload.profileID, protocolMajor: 1, localRole: .handy)
    try reject("invalid signed run") {
      _ = try SignedPairManifest.verify(signRaw(encode(payload)), trust: matchingTrust)
    }
  }
  for profileID in ["trial-20260905", "unified-candidate:other", "unified-candidate::trial-20260905",
                    "unified-candidate:trial-20260905/../daily"] {
    let payload = PairManifestPayload(keyID: trust.keyID, runID: trust.runID,
      profileID: profileID, protocolMajor: 1, peers: manifestPayload.peers)
    let matchingTrust = PairTrust(publicKeyX963: key.publicKeyX963, keyID: trust.keyID,
      runID: trust.runID, profileID: profileID, protocolMajor: 1, localRole: .handy)
    try reject("invalid signed derived profile") {
      _ = try SignedPairManifest.verify(signRaw(encode(payload)), trust: matchingTrust)
    }
  }
  for identifier in ["com.inputia:synthetic", "com.inputia.\" or true", "com.inputia/host"] {
    let peer = PairCodeIdentity(role: .inputia, identifier: identifier,
      cdhashes: manifestPayload.peers[1].cdhashes)
    let payload = PairManifestPayload(keyID: trust.keyID, runID: trust.runID,
      profileID: trust.profileID, protocolMajor: 1, peers: [manifestPayload.peers[0], peer])
    try reject("code identifier remains strict") {
      _ = try SignedPairManifest.verify(signRaw(encode(payload)), trust: trust)
    }
  }
  for (name, raw) in [
    ("duplicate payload", Data("{\"schemaVersion\":1,".utf8) + rawPayload.dropFirst()),
    ("unknown payload", Data("{\"unexpected\":true,".utf8) + rawPayload.dropFirst()),
  ] {
    try reject(name) { _ = try SignedPairManifest.verify(signRaw(raw), trust: trust) }
  }
  try reject("domain separation") {
    _ = try SignedPairManifest.verify(signRaw(rawPayload, domain: "other-domain"), trust: trust)
  }
  try reject("changed signed bytes") {
    var object = try JSONSerialization.jsonObject(with: signed) as! [String: String]
    object["payloadBase64"] = Data("{}".utf8).base64EncodedString()
    _ = try SignedPairManifest.verify(JSONSerialization.data(withJSONObject: object,
      options: [.sortedKeys, .withoutEscapingSlashes]), trust: trust)
  }
  try reject("wrong role set") {
    _ = try key.sign(PairManifestPayload(keyID: trust.keyID, runID: trust.runID,
      profileID: trust.profileID, protocolMajor: 1,
      peers: [manifestPayload.peers[0], manifestPayload.peers[0]]))
  }
  func exchange(_ executable: String, expected: Bool, hardened: Bool = false) throws {
    let path = root + "/\(UUID().uuidString.prefix(8)).sock"
    let server = try listener(path); defer { close(server); unlink(path) }
    let process = try child(executable, ["--fixture-client", path, manifestPath, pubPath])
    defer { stop(process) }
    try ready(server); let peer = accept(server, nil, nil)
    guard peer >= 0 else { try fail("accept") }; defer { close(peer) }
    let policy = try SignedPairManifest.verify(signed,
      trust: fixtureTrust(key.publicKeyX963, role: .handy, hardened: hardened))
    var verdict: UInt8 = 0
    do { _ = try PeerAuthenticator.authenticate(socketFD: peer, manifest: policy, expectedRole: .inputia); verdict = 1 }
    catch { verdict = 0 }
    try ready(peer); var clientVerdict: UInt8 = 0
    guard Darwin.read(peer, &clientVerdict, 1) == 1 else { try fail("client verdict") }
    _ = Darwin.write(peer, &verdict, 1)
    process.waitUntilExit()
    try require(verdict == (expected ? 1 : 0), "server peer authentication")
    if !hardened { try require(clientVerdict == (expected ? 1 : 0), "client peer authentication") }
    try require(process.terminationStatus == (expected ? 0 : 3), "child exit")
  }
  try exchange(host, expected: true)
  try exchange(rogue, expected: false)
  try exchange(host, expected: false, hardened: true)
  // 不可信程序抢先监听端点时，真实 Host 必须拒绝服务器，不能只验证客户端身份。
  let rogueSocket = root + "/rogue.sock"
  let server = Process(); let readiness = Pipe()
  server.executableURL = URL(fileURLWithPath: rogue)
  server.arguments = ["--fixture-server", rogueSocket, manifestPath, pubPath]
  server.standardOutput = readiness; server.standardError = FileHandle.nullDevice
  try server.run(); defer { stop(server) }
  try ready(readiness.fileHandleForReading.fileDescriptor)
  guard readiness.fileHandleForReading.readData(ofLength: 1) == Data([1]) else {
    try fail("rogue server startup")
  }
  let goodClient = try child(host, ["--fixture-client", rogueSocket, manifestPath, pubPath])
  defer { stop(goodClient) }
  goodClient.waitUntilExit(); server.waitUntilExit()
  try require(goodClient.terminationStatus == 3, "trusted client rejects rogue server")
  try require(server.terminationStatus == 3, "rogue cannot authenticate as local server")
  print("pair_auth_self_check=pass checks=\(checks) keychain_written=false private_key_persisted=false socket_fixture_only=true business_authentication_ready=false")
}

@main
struct PairAuthTool {
  static func main() {
    do {
      let args = Array(CommandLine.arguments.dropFirst())
      if args.count == 4, args[0] == "--fixture-client" {
        exit(try fixtureClient(Array(args.dropFirst())))
      }
      if args.count == 4, args[0] == "--fixture-server" {
        exit(try fixtureServer(Array(args.dropFirst())))
      }
      if args.count == 3, args[0] == "self-check" {
        try selfCheck(host: args[1], rogue: args[2]); return
      }
      if args.count == 3, args[0] == "keygen" {
        let key = try PairBuildKey()
        try writeNew(key.privateRepresentationForBuildOnly(), args[1])
        try writeNew(key.publicKeyX963, args[2])
        print("build_key_created=true keychain_written=false"); return
      }
      if args.count == 4, args[0] == "sign" {
        let key = try PairBuildKey(privateRepresentationForBuildOnly: read(args[1], limit: 97, privateKey: true))
        let raw = try read(args[2], limit: 8192)
        let payload = try JSONDecoder().decode(PairManifestPayload.self, from: raw)
        guard try encode(payload) == raw else { try fail("noncanonical signing input") }
        try writeNew(key.sign(payload), args[3]); print("manifest_signed=true"); return
      }
      if args.count == 3, args[0] == "identity", let role = PairRole(rawValue: args[1]) {
        print(String(decoding: try encode(identity(args[2], role: role)), as: UTF8.self)); return
      }
      if args.count == 6, args[0] == "bridge-fixture-manifest" {
        let key = try PairBuildKey(privateRepresentationForBuildOnly: read(args[1], limit: 97, privateKey: true))
        let handy = try identity(args[2], role: .handy)
        let host = try identity(args[3], role: .inputia)
        let weak = try identity(args[4], role: .inputia)
        guard host.identifier == weak.identifier else { try fail("fixture role identifiers") }
        let payload = PairManifestPayload(keyID: "bridge-fixture-key", runID: "trial-20260905",
          profileID: "unified-candidate:trial-20260905", protocolMajor: 1,
          peers: [handy, PairCodeIdentity(role: .inputia, identifier: host.identifier,
            cdhashes: host.cdhashes + weak.cdhashes)])
        try writeNew(key.sign(payload), args[5]); print("bridge_fixture_manifest_signed=true"); return
      }
      if args == ["build-marker"] {
        #if SYNTHETIC_ROGUE
        print("synthetic-untrusted-build")
        #else
        print("synthetic-trusted-build")
        #endif
        return
      }
      try fail("usage: keygen PRIVATE PUBLIC | sign PRIVATE PAYLOAD MANIFEST | identity ROLE BINARY | self-check HOST ROGUE")
    } catch {
      fputs("UnifiedPairAuthTool: \(error)\n", stderr); exit(2)
    }
  }
}
