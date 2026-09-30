// 独立签名合同回归；临时测试私钥仅在内存，不进入 Keychain。
import Foundation
import Security

@main
struct ReleasePairAuthCheck {
  static func main() throws {
    let key = try PairBuildKey()
    let payload = ReleasePairManifestPayload(productID: "com.inputia", releaseID: "inputia-contract-fixture",
      keyID: "fixture-v2", protocolMajor: 1, peers: [
        PairCodeIdentity(role: .handy, identifier: "com.inputia.Handy", cdhashes: [String(repeating: "a", count: 40)]),
        PairCodeIdentity(role: .inputia, identifier: "com.inputia.IME", cdhashes: [String(repeating: "b", count: 40)])])
    func trust(_ publicKey: Data? = nil, product: String = "com.inputia", release: String = "inputia-contract-fixture",
               keyID: String = "fixture-v2", major: Int = 1) -> PairReleaseTrust {
      PairReleaseTrust(publicKeyX963: publicKey ?? key.publicKeyX963, keyID: keyID,
        productID: product, releaseID: release, protocolMajor: major, localRole: .handy)
    }
    var checks = 0
    func require(_ condition: Bool, _ name: String) throws {
      guard condition else { throw PairAuthError.invalid("fixture: \(name)") }; checks += 1
    }
    func reject(_ name: String, _ operation: () throws -> Void) throws {
      do { try operation() } catch { checks += 1; return }
      throw PairAuthError.invalid("negative fixture accepted: \(name)")
    }
    let signed = try key.sign(payload)
    try require(try SignedPairManifest.verify(signed, trust: trust()).payload == payload, "v2 signature")
    for wrong in [trust(try PairBuildKey().publicKeyX963), trust(product: "other.product"),
                  trust(release: "inputia-other"), trust(keyID: "other-key"), trust(major: 2)] {
      try reject("wrong embedded binding") { _ = try SignedReleasePairManifest.verify(signed, trust: wrong) }
    }
    let privateData = try key.privateRepresentationForBuildOnly()
    let attributes: [String: Any] = [kSecAttrKeyType as String: kSecAttrKeyTypeECSECPrimeRandom,
      kSecAttrKeyClass as String: kSecAttrKeyClassPrivate, kSecAttrKeySizeInBits as String: 256]
    var error: Unmanaged<CFError>?
    guard let signingKey = SecKeyCreateWithData(privateData as CFData, attributes as CFDictionary, &error) else {
      throw PairAuthError.invalid("fixture key import")
    }
    func json(_ value: Any) throws -> Data {
      try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys, .withoutEscapingSlashes])
    }
    func signRaw(_ raw: Data, domain: String = "Inputia-Release-Pair-Manifest-v2\0") throws -> Data {
      guard let signature = SecKeyCreateSignature(signingKey, .ecdsaSignatureMessageX962SHA256,
        (Data(domain.utf8) + raw) as CFData, &error) else { throw PairAuthError.invalid("fixture raw sign") }
      return try json(["payloadBase64": raw.base64EncodedString(), "signatureBase64": (signature as Data).base64EncodedString()])
    }
    let raw = try payload.canonicalSigningBytes()
    let object = try JSONSerialization.jsonObject(with: raw) as! [String: Any]
    let patches: [[String: Any]] = [
      ["schemaVersion": 1], ["schemaVersion": true], ["schemaVersion": "2"], ["protocolMajor": 2],
      ["mode": "candidate"], ["channel": "stable"], ["profileID": "daily"], ["installationID": "local"],
      ["releaseID": "inputia-"], ["productID": "other.product"], ["releaseID": "inputia-other"], ["keyID": "wrong"],
      ["peers": [object["peers"] as! [[String: Any]]][0].prefix(1).map { $0 }],
    ]
    for patch in patches {
      var changed = object; changed.merge(patch) { _, new in new }
      try reject("signed invalid payload \(patch.keys)") {
        _ = try SignedReleasePairManifest.verify(signRaw(json(changed)), trust: trust())
      }
    }
    let peers = object["peers"] as! [[String: Any]]
    for patch: [String: Any] in [["role": "handy"], ["role": "alien"], ["identifier": "com.inputia.Handy"],
       ["identifier": "host\" or true"], ["cdhashes": []], ["cdhashes": [String(repeating: "b", count: 39)]],
       ["cdhashes": [String(repeating: "B", count: 40)]],
       ["cdhashes": [String(repeating: "b", count: 40), String(repeating: "b", count: 40)]], ["unexpected": true]] {
      var changedPeers = peers; changedPeers[1].merge(patch) { _, new in new }
      var changed = object; changed["peers"] = changedPeers
      try reject("signed invalid peer") { _ = try SignedReleasePairManifest.verify(signRaw(json(changed)), trust: trust()) }
    }
    for prefix in ["{\"schemaVersion\":2,", "{\"channel\":\"stable\","] {
      try reject("duplicate or unknown signed field") {
        _ = try SignedReleasePairManifest.verify(signRaw(Data(prefix.utf8) + raw.dropFirst()), trust: trust())
      }
    }
    try reject("legacy domain") {
      _ = try SignedReleasePairManifest.verify(signRaw(raw, domain: "Handy-Inputia-Candidate-Pair-Manifest-v1\0"), trust: trust())
    }
    for malformed in [Data(), Data("{}".utf8), Data(repeating: 0, count: 16_385), signed + Data([10])] {
      try reject("malformed envelope") { _ = try SignedReleasePairManifest.verify(malformed, trust: trust()) }
    }
    let envelope = try JSONSerialization.jsonObject(with: signed) as! [String: String]
    let repeated = "{\"payloadBase64\":\"\(envelope["payloadBase64"]!)\",\"payloadBase64\":\"\(envelope["payloadBase64"]!)\",\"signatureBase64\":\"\(envelope["signatureBase64"]!)\"}"
    try reject("duplicate envelope") { _ = try SignedReleasePairManifest.verify(Data(repeated.utf8), trust: trust()) }
    var tampered = envelope; tampered["payloadBase64"] = Data("{}".utf8).base64EncodedString()
    try reject("tampered signed bytes") { _ = try SignedReleasePairManifest.verify(json(tampered), trust: trust()) }
    var unknown = envelope; unknown["unexpected"] = "field"
    try reject("unknown envelope") { _ = try SignedReleasePairManifest.verify(json(unknown), trust: trust()) }

    // 由变更前的 v1 二进制生成、固定提交的旧字节；不能用当前 sign() 自证兼容。
    let fixtures = URL(fileURLWithPath: CommandLine.arguments[1])
    let legacyBytes = try Data(contentsOf: fixtures.appendingPathComponent("v1-frozen-manifest.fixture"))
    let hex = try String(contentsOf: fixtures.appendingPathComponent("v1-frozen-public-key.hex"), encoding: .utf8).trimmingCharacters(in: .whitespacesAndNewlines)
    var legacyKey = Data(); var offset = hex.startIndex
    while offset < hex.endIndex {
      let end = hex.index(offset, offsetBy: 2)
      guard let byte = UInt8(hex[offset..<end], radix: 16) else { throw PairAuthError.invalid("frozen public key") }
      legacyKey.append(byte); offset = end
    }
    let legacyTrust = PairTrust(publicKeyX963: legacyKey, keyID: "fixture-v1-frozen", runID: "trial-20260905",
      profileID: "unified-candidate:trial-20260905", protocolMajor: 1, localRole: .handy)
    let frozen = try SignedPairManifest.verify(legacyBytes, trust: legacyTrust)
    try require(frozen.payload.keyID == legacyTrust.keyID && frozen.payload.schemaVersion == 1, "frozen v1 bytes preserved")
    try reject("v2 never falls back to v1") {
      _ = try SignedPairManifest.verify(legacyBytes, trust: PairReleaseTrust(publicKeyX963: legacyKey,
        keyID: legacyTrust.keyID, productID: "com.inputia", releaseID: "inputia-contract-fixture", protocolMajor: 1, localRole: .handy))
    }
    try reject("v1 does not accept v2") { _ = try SignedPairManifest.verify(signed, trust: legacyTrust) }
    print("release_pair_contract=pass checks=\(checks) frozen_v1=true certificate_accessed=false")
  }
}
