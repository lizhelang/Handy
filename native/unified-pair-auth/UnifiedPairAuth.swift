import Darwin
import Foundation
import Security

public enum PairAuthError: Error, CustomStringConvertible {
  case invalid(String)
  public var description: String {
    switch self { case .invalid(let reason): return "pair authentication rejected: \(reason)" }
  }
}

public enum PairRole: String, Codable, CaseIterable { case handy, inputia }

public struct PairCodeIdentity: Codable, Equatable {
  public let role: PairRole
  public let identifier: String
  public let cdhashes: [String]
  public init(role: PairRole, identifier: String, cdhashes: [String]) {
    self.role = role; self.identifier = identifier; self.cdhashes = cdhashes
  }
}

public struct PairManifestPayload: Codable, Equatable {
  public let schemaVersion: Int
  public let mode: String
  public let keyID: String
  public let runID: String
  public let profileID: String
  public let protocolMajor: Int
  public let peers: [PairCodeIdentity]
  public init(keyID: String, runID: String, profileID: String, protocolMajor: Int,
              peers: [PairCodeIdentity]) {
    self.schemaVersion = 1; self.mode = "candidate"; self.keyID = keyID
    self.runID = runID; self.profileID = profileID; self.protocolMajor = protocolMajor
    self.peers = peers
  }
  public func canonicalSigningBytes() throws -> Data { try canonical(self) }
}

/// 必须来自签名前嵌入的构建配置，禁止从 manifest、握手或可写设置中建立信任根。
public struct PairTrust {
  public let publicKeyX963: Data
  public let keyID: String
  public let runID: String
  public let profileID: String
  public let protocolMajor: Int
  public let localRole: PairRole
  public let requireHardenedRuntime: Bool
  public init(publicKeyX963: Data, keyID: String, runID: String, profileID: String,
              protocolMajor: Int, localRole: PairRole, requireHardenedRuntime: Bool = true) {
    self.publicKeyX963 = publicKeyX963; self.keyID = keyID; self.runID = runID
    self.profileID = profileID; self.protocolMajor = protocolMajor; self.localRole = localRole
    self.requireHardenedRuntime = requireHardenedRuntime
  }
}

/// 可分发配对清单只描述不可变程序；本机 profile/installation 在认证后的握手绑定。
public struct ReleasePairManifestPayload: Codable, Equatable {
  public let schemaVersion: Int
  public let productID: String
  public let releaseID: String
  public let keyID: String
  public let protocolMajor: Int
  public let peers: [PairCodeIdentity]
  public init(productID: String, releaseID: String, keyID: String,
              protocolMajor: Int, peers: [PairCodeIdentity]) {
    self.schemaVersion = 2; self.productID = productID; self.releaseID = releaseID
    self.keyID = keyID; self.protocolMajor = protocolMajor; self.peers = peers
  }
  public func canonicalSigningBytes() throws -> Data { try canonical(self) }
}

/// 仅接受签名前嵌入的公开构建常量。安装收据不能提供公钥或任意 release 信任。
public struct PairReleaseTrust {
  public let publicKeyX963: Data
  public let keyID: String
  public let productID: String
  public let releaseID: String
  public let protocolMajor: Int
  public let localRole: PairRole
  public let requireHardenedRuntime: Bool
  public init(publicKeyX963: Data, keyID: String, productID: String, releaseID: String,
              protocolMajor: Int, localRole: PairRole, requireHardenedRuntime: Bool = true) {
    self.publicKeyX963 = publicKeyX963; self.keyID = keyID
    self.productID = productID; self.releaseID = releaseID; self.protocolMajor = protocolMajor
    self.localRole = localRole; self.requireHardenedRuntime = requireHardenedRuntime
  }
}

private struct PairEnvelope: Codable {
  let payloadBase64: String
  let signatureBase64: String
}

/// 私钥仅供离线构建；不持久化到 Keychain，也不与 Apple 发行签名身份混用。
public final class PairBuildKey {
  private let privateKey: SecKey
  public let publicKeyX963: Data
  public init() throws {
    var error: Unmanaged<CFError>?
    defer { error?.release() }
    let attributes: [String: Any] = [
      kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
      kSecAttrKeySizeInBits as String: 256,
      kSecAttrIsPermanent as String: false,
    ]
    guard let key = SecKeyCreateRandomKey(attributes as CFDictionary, &error),
          let publicKey = SecKeyCopyPublicKey(key),
          let exported = SecKeyCopyExternalRepresentation(publicKey, &error) else {
      throw PairAuthError.invalid("build key creation")
    }
    privateKey = key; publicKeyX963 = exported as Data
  }

  public func sign(_ payload: PairManifestPayload) throws -> Data {
    let raw = try canonical(payload)
    try validatePayload(payload)
    var error: Unmanaged<CFError>?
    defer { error?.release() }
    guard let signature = SecKeyCreateSignature(privateKey,
      .ecdsaSignatureMessageX962SHA256, signedMessage(raw) as CFData, &error) else {
      throw PairAuthError.invalid("manifest signing")
    }
    return try canonical(PairEnvelope(payloadBase64: raw.base64EncodedString(),
      signatureBase64: (signature as Data).base64EncodedString()))
  }

  public func sign(_ payload: ReleasePairManifestPayload) throws -> Data {
    try validateReleasePayload(payload)
    let raw = try canonical(payload)
    var error: Unmanaged<CFError>?
    defer { error?.release() }
    guard let signature = SecKeyCreateSignature(privateKey,
      .ecdsaSignatureMessageX962SHA256, releaseSignedMessage(raw) as CFData, &error) else {
      throw PairAuthError.invalid("release manifest signing")
    }
    return try canonical(PairEnvelope(payloadBase64: raw.base64EncodedString(),
      signatureBase64: (signature as Data).base64EncodedString()))
  }

  /// 专用临时 0600 文件可用于构建阶段跨进程交接；调用方不得输出返回值。
  public func privateRepresentationForBuildOnly() throws -> Data {
    var error: Unmanaged<CFError>?
    defer { error?.release() }
    guard let data = SecKeyCopyExternalRepresentation(privateKey, &error) else {
      throw PairAuthError.invalid("build key export")
    }
    return data as Data
  }

  public init(privateRepresentationForBuildOnly data: Data) throws {
    var error: Unmanaged<CFError>?
    defer { error?.release() }
    let attributes: [String: Any] = [
      kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
      kSecAttrKeyClass as String: kSecAttrKeyClassPrivate,
      kSecAttrKeySizeInBits as String: 256,
      kSecAttrIsPermanent as String: false,
    ]
    guard data.count == 97,
          let key = SecKeyCreateWithData(data as CFData, attributes as CFDictionary, &error),
          let publicKey = SecKeyCopyPublicKey(key),
          let exported = SecKeyCopyExternalRepresentation(publicKey, &error) else {
      throw PairAuthError.invalid("build key import")
    }
    privateKey = key; publicKeyX963 = exported as Data
  }
}

private func canonical<T: Encodable>(_ value: T) throws -> Data {
  let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
  return try encoder.encode(value)
}

private func signedMessage(_ raw: Data) -> Data {
  Data("Handy-Inputia-Candidate-Pair-Manifest-v1\0".utf8) + raw
}

private func releaseSignedMessage(_ raw: Data) -> Data {
  Data("Inputia-Release-Pair-Manifest-v2\0".utf8) + raw
}

private func validID(_ value: String) -> Bool {
  !value.isEmpty && value.utf8.count <= 128 && value.utf8.allSatisfy {
    (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0)
      || [45, 46, 95].contains($0)
  }
}

private func validCandidateRunID(_ value: String) -> Bool {
  !value.isEmpty && value.utf8.count <= 64 && value.utf8.allSatisfy {
    (48...57).contains($0) || (65...90).contains($0) || (97...122).contains($0)
      || [45, 95].contains($0)
  }
}

private func validatePayload(_ payload: PairManifestPayload) throws {
  guard payload.schemaVersion == 1, payload.mode == "candidate",
        validID(payload.keyID), validCandidateRunID(payload.runID),
        payload.profileID == "unified-candidate:\(payload.runID)",
        payload.protocolMajor == 1,
        payload.peers.count == 2,
        Set(payload.peers.map(\.role)) == Set(PairRole.allCases) else {
    throw PairAuthError.invalid("manifest contract")
  }
  try validatePeers(payload.peers)
}

private func validatePeers(_ peers: [PairCodeIdentity]) throws {
  guard peers.count == 2, Set(peers.map(\.role)) == Set(PairRole.allCases) else {
    throw PairAuthError.invalid("peer roles")
  }
  for peer in peers {
    guard validID(peer.identifier), (1...4).contains(peer.cdhashes.count),
          Set(peer.cdhashes).count == peer.cdhashes.count,
          peer.cdhashes.allSatisfy({ hash in
            hash.utf8.count == 40 && hash.utf8.allSatisfy {
              (48...57).contains($0) || (97...102).contains($0)
            }
          }) else { throw PairAuthError.invalid("code identity contract") }
  }
  guard peers[0].identifier != peers[1].identifier else {
    throw PairAuthError.invalid("roles share identifier")
  }
}

private func validateReleasePayload(_ payload: ReleasePairManifestPayload) throws {
  guard payload.schemaVersion == 2, validID(payload.productID), validID(payload.keyID),
        payload.releaseID.utf8.count > 8, payload.releaseID.utf8.count <= 192,
        payload.releaseID.hasPrefix("inputia-"),
        payload.releaseID.utf8.allSatisfy({ byte in
          (48...57).contains(byte) || (65...90).contains(byte) || (97...122).contains(byte)
            || [45, 46, 95].contains(byte)
        }), payload.protocolMajor == 1 else {
    throw PairAuthError.invalid("release manifest contract")
  }
  try validatePeers(payload.peers)
}

public struct SignedPairManifest {
  public let payload: PairManifestPayload
  public let trust: PairTrust
  private init(payload: PairManifestPayload, trust: PairTrust) {
    self.payload = payload; self.trust = trust
  }

  /// 验证原始 payload 签名之后才解码业务字段；canonical roundtrip 拒绝未知/重复字段。
  public static func verify(_ envelopeBytes: Data, trust: PairTrust) throws -> Self {
    guard !envelopeBytes.isEmpty, envelopeBytes.count <= 16_384,
          trust.publicKeyX963.count == 65, trust.publicKeyX963.first == 4 else {
      throw PairAuthError.invalid("manifest size or trust key")
    }
    let envelope = try JSONDecoder().decode(PairEnvelope.self, from: envelopeBytes)
    guard try canonical(envelope) == envelopeBytes,
          let raw = Data(base64Encoded: envelope.payloadBase64), raw.count <= 8192,
          let signature = Data(base64Encoded: envelope.signatureBase64),
          (8...80).contains(signature.count) else {
      throw PairAuthError.invalid("manifest encoding")
    }
    var error: Unmanaged<CFError>?
    defer { error?.release() }
    let attributes: [String: Any] = [
      kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
      kSecAttrKeyClass as String: kSecAttrKeyClassPublic,
      kSecAttrKeySizeInBits as String: 256,
    ]
    guard let key = SecKeyCreateWithData(trust.publicKeyX963 as CFData,
                                       attributes as CFDictionary, &error),
          SecKeyVerifySignature(key, .ecdsaSignatureMessageX962SHA256,
            signedMessage(raw) as CFData, signature as CFData, &error) else {
      throw PairAuthError.invalid("manifest signature")
    }
    let payload = try JSONDecoder().decode(PairManifestPayload.self, from: raw)
    guard try canonical(payload) == raw else {
      throw PairAuthError.invalid("noncanonical, duplicate or unknown payload fields")
    }
    try validatePayload(payload)
    guard payload.keyID == trust.keyID, payload.runID == trust.runID,
          payload.profileID == trust.profileID, payload.protocolMajor == trust.protocolMajor else {
      throw PairAuthError.invalid("manifest does not match embedded trust")
    }
    return Self(payload: payload, trust: trust)
  }

  public func identity(for role: PairRole) throws -> PairCodeIdentity {
    guard let result = payload.peers.first(where: { $0.role == role }) else {
      throw PairAuthError.invalid("missing peer role")
    }
    return result
  }

  /// 显式 v2 重载；v2 验签失败直接抛出，绝不尝试 v1。
  public static func verify(_ envelopeBytes: Data, trust: PairReleaseTrust) throws -> SignedReleasePairManifest {
    try SignedReleasePairManifest.verify(envelopeBytes, trust: trust)
  }
}

public struct SignedReleasePairManifest {
  public let payload: ReleasePairManifestPayload
  public let trust: PairReleaseTrust
  private init(payload: ReleasePairManifestPayload, trust: PairReleaseTrust) {
    self.payload = payload; self.trust = trust
  }
  public static func verify(_ envelopeBytes: Data, trust: PairReleaseTrust) throws -> Self {
    guard !envelopeBytes.isEmpty, envelopeBytes.count <= 16_384,
          trust.publicKeyX963.count == 65, trust.publicKeyX963.first == 4 else {
      throw PairAuthError.invalid("release manifest size or trust key")
    }
    let envelope = try JSONDecoder().decode(PairEnvelope.self, from: envelopeBytes)
    guard try canonical(envelope) == envelopeBytes,
          let raw = Data(base64Encoded: envelope.payloadBase64), raw.count <= 8192,
          let signature = Data(base64Encoded: envelope.signatureBase64),
          (8...80).contains(signature.count) else {
      throw PairAuthError.invalid("release manifest encoding")
    }
    var error: Unmanaged<CFError>?
    defer { error?.release() }
    let attributes: [String: Any] = [
      kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
      kSecAttrKeyClass as String: kSecAttrKeyClassPublic,
      kSecAttrKeySizeInBits as String: 256,
    ]
    guard let key = SecKeyCreateWithData(trust.publicKeyX963 as CFData,
                                       attributes as CFDictionary, &error),
          SecKeyVerifySignature(key, .ecdsaSignatureMessageX962SHA256,
            releaseSignedMessage(raw) as CFData, signature as CFData, &error) else {
      throw PairAuthError.invalid("release manifest signature")
    }
    let payload = try JSONDecoder().decode(ReleasePairManifestPayload.self, from: raw)
    guard try canonical(payload) == raw else {
      throw PairAuthError.invalid("noncanonical, duplicate or unknown release fields")
    }
    try validateReleasePayload(payload)
    guard payload.keyID == trust.keyID, payload.productID == trust.productID,
          payload.releaseID == trust.releaseID, payload.protocolMajor == trust.protocolMajor else {
      throw PairAuthError.invalid("release manifest does not match embedded trust")
    }
    return Self(payload: payload, trust: trust)
  }
  public func identity(for role: PairRole) throws -> PairCodeIdentity {
    guard let result = payload.peers.first(where: { $0.role == role }) else {
      throw PairAuthError.invalid("missing release peer role")
    }
    return result
  }
}

public struct VerifiedPeer {
  public let role: PairRole
  public let auditToken: Data
  public let uid: uid_t
  public let runID: String
  public let profileID: String
  /// false 仅是代码身份实验结果，不能用于开放产品业务。
  public let hardeningEnforced: Bool
}

/// 证明代码身份、UID、发布配对和收据期望路径；运行时 profile 仍需单独绑定。
public struct VerifiedReleasePeer {
  public let role: PairRole
  public let auditToken: Data
  public let uid: uid_t
  public let productID: String
  public let releaseID: String
  public let hardeningEnforced: Bool
}

public enum PeerAuthenticator {
  private static func requirement(_ identity: PairCodeIdentity) throws -> SecRequirement {
    let hashes = identity.cdhashes.map { "cdhash H\"\($0)\"" }.joined(separator: " or ")
    let expression = "identifier \"\(identity.identifier)\" and (\(hashes))"
    var requirement: SecRequirement?
    guard SecRequirementCreateWithString(expression as CFString, [], &requirement) == errSecSuccess,
          let requirement else { throw PairAuthError.invalid("code requirement") }
    return requirement
  }

  /// 在连接双方的后台队列执行；每次连接认证并核验自身身份，不能从握手接受 role。
  public static func authenticate(socketFD: Int32, manifest: SignedPairManifest,
                                  expectedRole: PairRole) throws -> VerifiedPeer {
    let (audit, uid) = try authenticateCode(socketFD: socketFD,
      localRole: manifest.trust.localRole, localIdentity: manifest.identity(for: manifest.trust.localRole),
      peerIdentity: manifest.identity(for: expectedRole), expectedRole: expectedRole,
      hardened: manifest.trust.requireHardenedRuntime)
    return VerifiedPeer(role: expectedRole, auditToken: audit, uid: uid,
      runID: manifest.payload.runID, profileID: manifest.payload.profileID,
      hardeningEnforced: manifest.trust.requireHardenedRuntime)
  }

  public static func authenticate(socketFD: Int32, manifest: SignedReleasePairManifest,
                                  expectedRole: PairRole, expectedBundlePath: String) throws -> VerifiedReleasePeer {
    let (audit, uid) = try authenticateCode(socketFD: socketFD,
      localRole: manifest.trust.localRole, localIdentity: manifest.identity(for: manifest.trust.localRole),
      peerIdentity: manifest.identity(for: expectedRole), expectedRole: expectedRole,
      hardened: manifest.trust.requireHardenedRuntime, expectedBundlePath: expectedBundlePath)
    return VerifiedReleasePeer(role: expectedRole, auditToken: audit, uid: uid,
      productID: manifest.payload.productID, releaseID: manifest.payload.releaseID,
      hardeningEnforced: manifest.trust.requireHardenedRuntime)
  }

  private static func authenticateCode(socketFD: Int32, localRole: PairRole,
                                       localIdentity: PairCodeIdentity, peerIdentity: PairCodeIdentity,
                                       expectedRole: PairRole, hardened: Bool,
                                       expectedBundlePath: String? = nil) throws -> (Data, uid_t) {
    guard expectedRole != localRole else {
      throw PairAuthError.invalid("unexpected local role")
    }
    var local: SecCode?
    let localRequirement = try requirement(localIdentity)
    guard SecCodeCopySelf([], &local) == errSecSuccess, let local,
          SecCodeCheckValidity(local, [], localRequirement) == errSecSuccess else {
      throw PairAuthError.invalid("local code identity")
    }
    var socketType: Int32 = 0; var typeLength = socklen_t(MemoryLayout<Int32>.size)
    guard getsockopt(socketFD, SOL_SOCKET, SO_TYPE, &socketType, &typeLength) == 0,
          typeLength == MemoryLayout<Int32>.size, socketType == SOCK_STREAM else {
      throw PairAuthError.invalid("connected stream required")
    }
    var uid: uid_t = 0; var gid: gid_t = 0
    guard getpeereid(socketFD, &uid, &gid) == 0, uid == geteuid() else {
      throw PairAuthError.invalid("kernel peer uid")
    }
    var token = audit_token_t(); var length = socklen_t(MemoryLayout<audit_token_t>.size)
    guard getsockopt(socketFD, 0, LOCAL_PEERTOKEN, &token, &length) == 0,
          length == MemoryLayout<audit_token_t>.size else {
      throw PairAuthError.invalid("kernel peer audit token")
    }
    let audit = withUnsafeBytes(of: &token) { Data($0) }
    var code: SecCode?
    let peerRequirement = try requirement(peerIdentity)
    guard SecCodeCopyGuestWithAttributes(nil,
      [kSecGuestAttributeAudit as String: audit] as CFDictionary, [], &code) == errSecSuccess,
      let code,
      SecCodeCheckValidity(code, [], peerRequirement) == errSecSuccess else {
      throw PairAuthError.invalid("dynamic peer code requirement")
    }
    if hardened {
      try checkHardening(local, requirement: localRequirement)
      try checkHardening(code, requirement: peerRequirement)
    }
    if let expectedBundlePath {
      try checkBundlePath(code, expected: expectedBundlePath)
      // 路径元数据不替代动态身份，检查后再次确认同一 audit token 对象仍有效。
      guard SecCodeCheckValidity(code, [], peerRequirement) == errSecSuccess else {
        throw PairAuthError.invalid("dynamic peer changed during path verification")
      }
    }
    return (audit, uid)
  }

  private static func canonicalExistingPath(_ path: String) throws -> String {
    guard path.hasPrefix("/"), path.utf8.count < Int(PATH_MAX), !path.utf8.contains(0),
          !path.hasSuffix("/"), !path.contains("//"),
          !path.split(separator: "/").contains(where: { $0 == "." || $0 == ".." }) else {
      throw PairAuthError.invalid("noncanonical expected bundle path")
    }
    var resolved = [CChar](repeating: 0, count: Int(PATH_MAX))
    guard realpath(path, &resolved) != nil, String(cString: resolved) == path else {
      throw PairAuthError.invalid("bundle path contains symlink or does not exist")
    }
    // realpath 的相等检查已排除别名；逐级 lstat 同样拒绝当前路径链中的符号链接。
    var prefix = ""
    for component in path.split(separator: "/") {
      prefix += "/" + component
      var metadata = stat()
      guard lstat(prefix, &metadata) == 0, metadata.st_mode & S_IFMT != S_IFLNK else {
        throw PairAuthError.invalid("bundle path component changed")
      }
    }
    return path
  }

  private static func checkBundlePath(_ code: SecCode, expected: String) throws {
    let expected = try canonicalExistingPath(expected)
    var staticCode: SecStaticCode?
    var path: CFURL?
    guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode,
          SecCodeCopyPath(staticCode, [], &path) == errSecSuccess, let path else {
      throw PairAuthError.invalid("audit-token code bundle path")
    }
    let actual = (path as URL).path
    guard try canonicalExistingPath(actual) == expected else {
      throw PairAuthError.invalid("peer is outside the receipt component path")
    }
  }

  /// 此处为元数据门禁，不能取代候选依赖加载和注入负例的运行验证。
  private static func checkHardening(_ code: SecCode, requirement: SecRequirement) throws {
    var staticCode: SecStaticCode?
    guard SecCodeCopyStaticCode(code, [], &staticCode) == errSecSuccess, let staticCode,
          SecStaticCodeCheckValidity(staticCode, [], requirement) == errSecSuccess else {
      throw PairAuthError.invalid("hardening metadata")
    }
    var information: CFDictionary?
    guard SecCodeCopySigningInformation(staticCode,
      SecCSFlags(rawValue: kSecCSSigningInformation), &information) == errSecSuccess,
      let info = information as? [String: Any],
      let flags = info[kSecCodeInfoFlags as String] as? NSNumber,
      flags.uint32Value & 0x10000 != 0 else {
      throw PairAuthError.invalid("hardened runtime required")
    }
    let entitlements = info[kSecCodeInfoEntitlementsDict as String] as? [String: Any] ?? [:]
    for key in ["com.apple.security.get-task-allow", "get-task-allow",
                "com.apple.security.cs.disable-library-validation",
                "com.apple.security.cs.allow-dyld-environment-variables",
                "com.apple.security.cs.disable-executable-page-protection",
                "com.apple.security.cs.allow-unsigned-executable-memory"] {
      if let value = entitlements[key], (value as? Bool) != false {
        throw PairAuthError.invalid("unsafe runtime entitlement")
      }
    }
    // 再次绑定动态对象，不能依赖上面取得的静态路径作为身份根。
    guard SecCodeCheckValidity(code, [], requirement) == errSecSuccess else {
      throw PairAuthError.invalid("dynamic code became invalid")
    }
  }
}
