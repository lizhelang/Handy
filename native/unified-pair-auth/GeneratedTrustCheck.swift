// 由脚本生成的编译期公钥校验真实 CLI 产物；不从待验清单提取信任根。
import Foundation

@main
struct GeneratedTrustCheck {
  static func main() throws {
    let bytes = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1]))
    let manifest = try SignedPairManifest.verify(bytes, trust: InputiaEmbeddedPairTrust.trust)
    print("generated_pair_trust=pass schema=\(manifest.payload.schemaVersion)")
  }
}
