import Foundation

private final class BridgeManifest {
  private enum Binding {
    case legacy(SignedPairManifest)
    case release(SignedReleasePairManifest)
  }
  private let binding: Binding
  init(_ manifest: SignedPairManifest) { binding = .legacy(manifest) }
  init(_ manifest: SignedReleasePairManifest) { binding = .release(manifest) }
  func authenticate(descriptor: Int32, role: PairRole, expectedPath: String? = nil) throws -> (Data, UInt32, PairRole, Bool) {
    switch binding {
    case .legacy(let manifest):
      guard expectedPath == nil else { throw PairAuthError.invalid("v2 entry requires v2 manifest") }
      let peer = try PeerAuthenticator.authenticate(socketFD: descriptor, manifest: manifest, expectedRole: role)
      return (peer.auditToken, peer.uid, peer.role, peer.hardeningEnforced)
    case .release(let manifest):
      guard let expectedPath else { throw PairAuthError.invalid("release receipt path required") }
      let peer = try PeerAuthenticator.authenticate(socketFD: descriptor, manifest: manifest, expectedRole: role, expectedBundlePath: expectedPath)
      return (peer.auditToken, peer.uid, peer.role, peer.hardeningEnforced)
    }
  }
}

@_cdecl("uipa_manifest_load_v2")
public func bridgeReleaseManifestLoad(
  _ envelope: UnsafePointer<UInt8>?, _ envelopeLength: Int,
  _ publicKey: UnsafePointer<UInt8>?, _ publicKeyLength: Int,
  _ keyID: UnsafePointer<UInt8>?, _ keyIDLength: Int,
  _ productID: UnsafePointer<UInt8>?, _ productIDLength: Int,
  _ releaseID: UnsafePointer<UInt8>?, _ releaseIDLength: Int,
  _ protocolMajor: UInt32, _ localRole: UInt32,
  _ output: UnsafeMutablePointer<UnsafeMutableRawPointer?>?
) -> Int32 {
  guard let output else { return 1 }
  output.pointee = nil
  guard let role = bridgeRole(localRole), protocolMajor == 1,
        let envelope = bridgeData(envelope, envelopeLength, maximum: 16_384),
        publicKeyLength == 65, let publicKey = bridgeData(publicKey, publicKeyLength, maximum: 65),
        let keyBytes = bridgeData(keyID, keyIDLength, maximum: 128),
        let productBytes = bridgeData(productID, productIDLength, maximum: 128),
        let releaseBytes = bridgeData(releaseID, releaseIDLength, maximum: 192),
        let keyID = String(data: keyBytes, encoding: .utf8),
        let productID = String(data: productBytes, encoding: .utf8),
        let releaseID = String(data: releaseBytes, encoding: .utf8) else { return 1 }
  do {
    let trust = PairReleaseTrust(publicKeyX963: publicKey, keyID: keyID,
      productID: productID, releaseID: releaseID, protocolMajor: Int(protocolMajor),
      localRole: role, requireHardenedRuntime: true)
    let manifest = try SignedReleasePairManifest.verify(envelope, trust: trust)
    output.pointee = Unmanaged.passRetained(BridgeManifest(manifest)).toOpaque()
    return 0
  } catch { return 2 }
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
  authenticateBridge(handle, descriptor, expectedRole, nil, output)
}

@_cdecl("uipa_authenticate_v2")
public func bridgeReleaseAuthenticate(_ handle: UnsafeMutableRawPointer?, _ descriptor: Int32,
                                      _ expectedRole: UInt32,
                                      _ expectedPath: UnsafePointer<UInt8>?, _ pathLength: Int,
                                      _ output: UnsafeMutablePointer<UipaVerifiedPeer>?) -> Int32 {
  output?.pointee = UipaVerifiedPeer()
  guard let data = bridgeData(expectedPath, pathLength, maximum: 4095),
        let path = String(data: data, encoding: .utf8), !path.utf8.contains(0) else { return 1 }
  return authenticateBridge(handle, descriptor, expectedRole, path, output)
}

private func authenticateBridge(_ handle: UnsafeMutableRawPointer?, _ descriptor: Int32,
                                _ expectedRole: UInt32, _ expectedPath: String?,
                                _ output: UnsafeMutablePointer<UipaVerifiedPeer>?) -> Int32 {
  guard let output else { return 1 }
  output.pointee = UipaVerifiedPeer()
  guard let handle, descriptor >= 0, let role = bridgeRole(expectedRole) else { return 1 }
  let manifest = Unmanaged<BridgeManifest>.fromOpaque(handle).takeUnretainedValue()
  do {
    let (auditToken, uid, verifiedRole, hardeningEnforced) = try manifest.authenticate(
      descriptor: descriptor, role: role, expectedPath: expectedPath)
    guard hardeningEnforced, auditToken.count == 32 else { return 3 }
    _ = withUnsafeMutableBytes(of: &output.pointee.audit_token) { bytes in
      auditToken.copyBytes(to: bytes)
    }
    output.pointee.uid = uid
    output.pointee.role = verifiedRole == .handy ? 1 : 2
    return 0
  } catch { return 3 }
}

@_cdecl("uipa_manifest_free")
public func bridgeManifestFree(_ handle: UnsafeMutableRawPointer?) {
  if let handle { Unmanaged<BridgeManifest>.fromOpaque(handle).release() }
}
