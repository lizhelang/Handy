import Foundation
import Security

/// 安装器只读校验：用现有配对信任验证新清单及两份程序，不申请权限。
@main struct CandidateUpdateVerify {
  struct Metadata: Decodable {
    let run_id: String
    let profile_id: String
    let key_id: String
    let public_key_x963_hex: String
  }
  static func main() throws {
    let args = CommandLine.arguments
    guard args.count == 7 else { throw PairAuthError.invalid("verifier arguments") }
    let metadata = try JSONDecoder().decode(Metadata.self, from: Data(contentsOf: URL(fileURLWithPath: args[1])))
    guard metadata.run_id == args[6], metadata.profile_id == "unified-candidate:" + args[6],
      metadata.public_key_x963_hex.count == 130 else { throw PairAuthError.invalid("profile") }
    let chars = Array(metadata.public_key_x963_hex)
    var bytes = [UInt8]()
    for index in stride(from: 0, to: chars.count, by: 2) {
      guard let byte = UInt8(String(chars[index...index+1]), radix: 16) else { throw PairAuthError.invalid("public key") }
      bytes.append(byte)
    }
    let trust = PairTrust(publicKeyX963: Data(bytes), keyID: metadata.key_id,
      runID: metadata.run_id, profileID: metadata.profile_id, protocolMajor: 1, localRole: .inputia)
    _ = try SignedPairManifest.verify(Data(contentsOf: URL(fileURLWithPath: args[2])), trust: trust)
    let manifest = try SignedPairManifest.verify(Data(contentsOf: URL(fileURLWithPath: args[3])), trust: trust)
    for (role, path) in [(PairRole.handy, args[4]), (PairRole.inputia, args[5])] {
      let identity = try manifest.identity(for: role)
      let hashes = identity.cdhashes.map { "cdhash H\"\($0)\"" }.joined(separator: " or ")
      var requirement: SecRequirement?
      guard SecRequirementCreateWithString("identifier \"\(identity.identifier)\" and (\(hashes))" as CFString, [], &requirement) == errSecSuccess,
        let requirement else { throw PairAuthError.invalid("requirement") }
      var code: SecStaticCode?
      guard SecStaticCodeCreateWithPath(URL(fileURLWithPath:path) as CFURL, [], &code) == errSecSuccess,
        let code, SecStaticCodeCheckValidity(code, SecCSFlags(rawValue:kSecCSCheckAllArchitectures | kSecCSStrictValidate), requirement) == errSecSuccess
      else { throw PairAuthError.invalid("updated app identity") }
      var info: CFDictionary?
      guard SecCodeCopySigningInformation(code, SecCSFlags(rawValue:kSecCSSigningInformation), &info) == errSecSuccess,
        let flags = (info as? [String: Any])?[kSecCodeInfoFlags as String] as? NSNumber,
        flags.uint32Value & 0x10000 != 0 else { throw PairAuthError.invalid("runtime protection") }
    }
    print("candidatePairUpdateVerified=true")
  }
}
