import Foundation

private final class BridgeManifest {
  let manifest: SignedPairManifest
  init(_ manifest: SignedPairManifest) { self.manifest = manifest }
}

private func bridgeRole(_ value: UInt32) -> PairRole? {
  switch value { case 1: return .handy; case 2: return .inputia; default: return nil }
}

private func bridgeData(_ pointer: UnsafePointer<UInt8>?, _ count: Int, maximum: Int) -> Data? {
  guard let pointer, count > 0, count <= maximum else { return nil }
  return Data(bytes: pointer, count: count)
}

@_cdecl("uipa_manifest_load")
public func bridgeManifestLoad(
  _ envelope: UnsafePointer<UInt8>?, _ envelopeLength: Int,
  _ publicKey: UnsafePointer<UInt8>?, _ publicKeyLength: Int,
  _ keyID: UnsafePointer<UInt8>?, _ keyIDLength: Int,
  _ runID: UnsafePointer<UInt8>?, _ runIDLength: Int,
  _ profileID: UnsafePointer<UInt8>?, _ profileIDLength: Int,
  _ localRole: UInt32, _ output: UnsafeMutablePointer<UnsafeMutableRawPointer?>?
) -> Int32 {
  guard let output else { return 1 }
  output.pointee = nil
  guard let role = bridgeRole(localRole),
        let envelope = bridgeData(envelope, envelopeLength, maximum: 16_384),
        publicKeyLength == 65, let publicKey = bridgeData(publicKey, publicKeyLength, maximum: 65),
        let keyBytes = bridgeData(keyID, keyIDLength, maximum: 128),
        let runBytes = bridgeData(runID, runIDLength, maximum: 64),
        let profileBytes = bridgeData(profileID, profileIDLength, maximum: 128),
        let keyID = String(data: keyBytes, encoding: .utf8),
        let runID = String(data: runBytes, encoding: .utf8),
        let profileID = String(data: profileBytes, encoding: .utf8) else { return 1 }
  do {
    let trust = PairTrust(publicKeyX963: publicKey, keyID: keyID, runID: runID,
      profileID: profileID, protocolMajor: 1, localRole: role, requireHardenedRuntime: true)
    let manifest = try SignedPairManifest.verify(envelope, trust: trust)
    output.pointee = Unmanaged.passRetained(BridgeManifest(manifest)).toOpaque()
    return 0
  } catch { return 2 }
}

@_cdecl("uipa_authenticate")
public func bridgeAuthenticate(_ handle: UnsafeMutableRawPointer?, _ descriptor: Int32,
                               _ expectedRole: UInt32,
                               _ output: UnsafeMutablePointer<UipaVerifiedPeer>?) -> Int32 {
  guard let output else { return 1 }
  output.pointee = UipaVerifiedPeer()
  guard let handle, descriptor >= 0, let role = bridgeRole(expectedRole) else { return 1 }
  let manifest = Unmanaged<BridgeManifest>.fromOpaque(handle).takeUnretainedValue().manifest
  do {
    let verified = try PeerAuthenticator.authenticate(socketFD: descriptor,
      manifest: manifest, expectedRole: role)
    guard verified.hardeningEnforced, verified.auditToken.count == 32 else { return 3 }
    _ = withUnsafeMutableBytes(of: &output.pointee.audit_token) { bytes in
      verified.auditToken.copyBytes(to: bytes)
    }
    output.pointee.uid = verified.uid
    output.pointee.role = verified.role == .handy ? 1 : 2
    return 0
  } catch { return 3 }
}

@_cdecl("uipa_manifest_free")
public func bridgeManifestFree(_ handle: UnsafeMutableRawPointer?) {
  if let handle { Unmanaged<BridgeManifest>.fromOpaque(handle).release() }
}
