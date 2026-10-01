import Foundation
import Security

/// 只读 v2 发布配对验签器：不安装、不改输入源、不读取私钥。
@main struct ReleasePairAuthVerify {
  struct Metadata: Decodable {
    let product_id: String
    let release_id: String
    let key_id: String
    let public_key_x963_hex: String
  }

  static func run(_ path: String, identity: PairCodeIdentity) throws {
    let hashes = identity.cdhashes.map { "cdhash H\"\($0)\"" }.joined(separator: " or ")
    var requirement: SecRequirement?
    guard SecRequirementCreateWithString("identifier \"\(identity.identifier)\" and (\(hashes))" as CFString, [], &requirement) == errSecSuccess,
          let requirement else { throw PairAuthError.invalid("release requirement") }
    var code: SecStaticCode?
    guard SecStaticCodeCreateWithPath(URL(fileURLWithPath: path) as CFURL, [], &code) == errSecSuccess,
          let code,
          SecStaticCodeCheckValidity(code, SecCSFlags(rawValue: kSecCSCheckAllArchitectures | kSecCSStrictValidate), requirement) == errSecSuccess
    else { throw PairAuthError.invalid("release peer identity") }
  }

  static func main() throws {
    let args = CommandLine.arguments
    guard args.count == 6 else { throw PairAuthError.invalid("usage: metadata public-build pair-manifest control-app inputia-app") }
    let metadata = try JSONDecoder().decode(Metadata.self, from: Data(contentsOf: URL(fileURLWithPath: args[1])))
    let publicBuild = try JSONDecoder().decode([String: AnyDecodable].self, from: Data(contentsOf: URL(fileURLWithPath: args[2])))
    guard publicBuild["schema_version"]?.intValue == 2,
          publicBuild["product_id"]?.stringValue == metadata.product_id,
          publicBuild["release_id"]?.stringValue == metadata.release_id else {
      throw PairAuthError.invalid("public build metadata")
    }
    let chars = Array(metadata.public_key_x963_hex)
    guard chars.count == 130 else { throw PairAuthError.invalid("public key") }
    var bytes = [UInt8]()
    for index in stride(from: 0, to: chars.count, by: 2) {
      guard let byte = UInt8(String(chars[index...index + 1]), radix: 16) else { throw PairAuthError.invalid("public key") }
      bytes.append(byte)
    }
    let trust = PairReleaseTrust(publicKeyX963: Data(bytes), keyID: metadata.key_id,
      productID: metadata.product_id, releaseID: metadata.release_id, protocolMajor: 1, localRole: .inputia)
    let manifest = try SignedReleasePairManifest.verify(Data(contentsOf: URL(fileURLWithPath: args[3])), trust: trust)
    try run(args[4], identity: manifest.identity(for: .handy))
    try run(args[5], identity: manifest.identity(for: .inputia))
    print("releasePairVerified=true releaseID=\(manifest.payload.releaseID)")
  }
}

struct AnyDecodable: Decodable {
  let stringValue: String?
  let intValue: Int?
  init(from decoder: Decoder) throws {
    let container = try decoder.singleValueContainer()
    stringValue = try? container.decode(String.self)
    intValue = try? container.decode(Int.self)
  }
}
