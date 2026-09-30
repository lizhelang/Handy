import Darwin
import Foundation
import MachO

@_silgen_name("inputia_session_new_with_paths")
private func runtimeProbeNew(_ schema: UnsafePointer<CChar>, _ library: UnsafePointer<CChar>, _ shared: UnsafePointer<CChar>, _ user: UnsafePointer<CChar>, _ page: Int) -> UnsafeMutableRawPointer?
@_silgen_name("inputia_session_free")
private func runtimeProbeFree(_ session: UnsafeMutableRawPointer?)
@_silgen_name("inputia_session_set_input_mode")
private func runtimeProbeMode(_ session: UnsafeMutableRawPointer?, _ mode: Int32) -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_session_handle_char")
private func runtimeProbeChar(_ session: UnsafeMutableRawPointer?, _ scalar: UInt32) -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_session_handle_special")
private func runtimeProbeSpecial(_ session: UnsafeMutableRawPointer?, _ key: Int32) -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_string_free")
private func runtimeProbeStringFree(_ pointer: UnsafeMutablePointer<CChar>?)

enum InputiaRuntimeDiagnostics {
  private struct Failure: Error { let reason: String }
  private static func require(_ value: Bool, _ reason: String) throws {
    if !value { throw Failure(reason: reason) }
  }
  private static func json(_ pointer: UnsafeMutablePointer<CChar>?) throws -> [String: Any] {
    guard let pointer else { throw Failure(reason: "missing CAPI response") }
    defer { runtimeProbeStringFree(pointer) }
    guard let data = String(cString: pointer).data(using: .utf8),
          let value = try JSONSerialization.jsonObject(with: data) as? [String: Any],
          value["ok"] as? Bool == true else { throw Failure(reason: "CAPI rejected fixture input") }
    return value
  }

  /// 在IMKServer/窗口/全局按键监视器创建之前运行；不使用真实用户词典或系统剪贴板。
  static func run() {
    do {
      try require(InputiaProfile.current.isCandidate, "explicit candidate bundle required")
      #if INPUTIA_PAIRED_BUILD
      let trust = InputiaEmbeddedPairTrust.trust
      #if INPUTIA_RELEASE_PAIR_V2
      try require(InputiaProfile.current.pairBinding?.pair_release_id == trust.releaseID && trust.requireHardenedRuntime,
        "embedded release pair identity mismatch")
      print("inputia_embedded_pair_key_id=\(trust.keyID) release_id=\(trust.releaseID) runtime_key_configuration=false")
      #else
      try require(trust.runID == InputiaProfile.current.runID && trust.requireHardenedRuntime,
                  "embedded pair build identity mismatch")
      print("inputia_embedded_pair_key_id=\(trust.keyID) profile_id=\(trust.profileID) runtime_key_configuration=false")
      #endif
      #endif
      guard let resources = Bundle.main.resourceURL else { throw Failure(reason: "bundle resources missing") }
      let shared = resources.appendingPathComponent("RimeData", isDirectory: true)
      try require(FileManager.default.fileExists(atPath: shared.appendingPathComponent("double_pinyin_flypy.schema.yaml").path), "bundled schemas unavailable")
      let root = FileManager.default.temporaryDirectory.resolvingSymlinksInPath()
        .appendingPathComponent("InputiaRuntimeCheck-\(UUID().uuidString)", isDirectory: true)
      try FileManager.default.createDirectory(at: root, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
      defer { try? FileManager.default.removeItem(at: root) }
      for (schema, keys) in [("luna_pinyin_simp", "zhongguo"), ("double_pinyin_flypy", "vsgo")] {
        let user = root.appendingPathComponent(schema, isDirectory: true)
        let session = schema.withCString { schema in
          "/synthetic/no-dynamic-rime-fallback.dylib".withCString { library in
            shared.path.withCString { shared in user.path.withCString { user in runtimeProbeNew(schema, library, shared, user, 7) } }
          }
        }
        guard let session else { throw Failure(reason: "static bundled session failed") }
        do {
          defer { runtimeProbeFree(session) }
          _ = try json(runtimeProbeMode(session, 2))
          for scalar in keys.unicodeScalars { _ = try json(runtimeProbeChar(session, scalar.value)) }
          let commit = try json(runtimeProbeSpecial(session, 3))
          try require(commit["commit"] as? String == "中国", "bundled pinyin commit mismatch")
        }
      }
      for index in 0..<_dyld_image_count() {
        guard let name = _dyld_get_image_name(index) else { continue }
        let path = String(cString: name).lowercased()
        try require(!path.contains("librime") && !path.contains("squirrel.app") && !path.contains("/opt/homebrew/"), "external Rime/runtime image loaded")
      }
      print("inputia_candidate_runtime_check=pass bundled_static_rime=true pinyin_commit=true double_pinyin_commit=true external_librime_loaded=false imk_server_started=false daily_user_dictionary_opened=false synthetic_profile=true")
    } catch {
      fputs("inputia_candidate_runtime_check=failed reason=\(error)\n", stderr)
      exit(1)
    }
  }
}
