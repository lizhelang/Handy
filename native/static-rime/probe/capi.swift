import Darwin
import Foundation
import MachO

@_silgen_name("inputia_session_new_from_settings")
private func newSession(_ path: UnsafePointer<CChar>) -> UnsafeMutableRawPointer?
@_silgen_name("inputia_session_free")
private func freeSession(_ session: UnsafeMutableRawPointer?)
@_silgen_name("inputia_session_set_input_mode")
private func setMode(_ session: UnsafeMutableRawPointer?, _ mode: Int32) -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_session_handle_char")
private func handleChar(_ session: UnsafeMutableRawPointer?, _ scalar: UInt32) -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_session_handle_special")
private func handleSpecial(_ session: UnsafeMutableRawPointer?, _ key: Int32) -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_session_learn")
private func learn(_ session: UnsafeMutableRawPointer?, _ source: Int32, _ text: UnsafePointer<CChar>, _ bundle: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_session_voice_hotwords")
private func hotwords(_ session: UnsafeMutableRawPointer?, _ limit: Int) -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_string_free")
private func freeString(_ value: UnsafeMutablePointer<CChar>?)
@_silgen_name("rime_get_api")
private func staticRimeAPI() -> UnsafeMutableRawPointer?

private struct ProbeFailure: Error { let message: String }
private func require(_ value: Bool, _ reason: String) throws {
  if !value { throw ProbeFailure(message: reason) }
}
private func json(_ pointer: UnsafeMutablePointer<CChar>?) throws -> [String: Any] {
  guard let pointer else { throw ProbeFailure(message: "CAPI returned null JSON") }
  defer { freeString(pointer) }
  guard let value = try JSONSerialization.jsonObject(with: Data(String(cString: pointer).utf8)) as? [String: Any] else {
    throw ProbeFailure(message: "invalid CAPI JSON")
  }
  return value
}
private func verifyImages() throws {
  let own = URL(fileURLWithPath: CommandLine.arguments[0]).resolvingSymlinksInPath().path
  for index in 0..<_dyld_image_count() {
    guard let pointer = _dyld_get_image_name(index) else { continue }
    let path = String(cString: pointer)
    try require(path.hasPrefix("/usr/lib/") || path.hasPrefix("/System/Library/") || URL(fileURLWithPath: path).resolvingSymlinksInPath().path == own,
                "unexpected external image: \(path)")
  }
  var info = Dl_info()
  try require(staticRimeAPI().map { dladdr($0, &info) != 0 } == true, "static API address unavailable")
  try require(info.dli_fname.map { URL(fileURLWithPath: String(cString: $0)).resolvingSymlinksInPath().path == own } == true,
              "Rime API table is not in the executable")
}
private func input(_ session: UnsafeMutableRawPointer, _ keys: String) throws {
  _ = try json(setMode(session, 2))
  var result: [String: Any] = [:]
  for scalar in keys.unicodeScalars { result = try json(handleChar(session, scalar.value)) }
  let candidates = result["visible_candidates"] as? [[String: Any]]
  try require(candidates?.first?["text"] as? String == "中国", "wrong candidate for \(keys)")
  let commit = try json(handleSpecial(session, 3))
  try require(commit["commit"] as? String == "中国", "wrong commit for \(keys)")
}

@main
private struct CAPIStaticProbe {
  static func main() {
    do {
      try require(CommandLine.arguments.count == 3, "usage: CAPIStaticProbe CANDIDATE_RIME_DATA NEW_RUN_DIR")
      let data = URL(fileURLWithPath: CommandLine.arguments[1]).standardizedFileURL
      let run = URL(fileURLWithPath: CommandLine.arguments[2]).standardizedFileURL
      let user = run.appendingPathComponent("user")
      try require(!FileManager.default.fileExists(atPath: user.path), "refusing reused profile")
      try FileManager.default.createDirectory(at: user, withIntermediateDirectories: false)
      let settings = run.appendingPathComponent("settings.json")
      var document: [String: Any] = [
        "schema_id": "luna_pinyin_simp", "candidate_page_size": 5,
        "rime_shared_data_dir": data.path, "rime_user_data_dir": user.path,
        "rime_dylib_path": "/synthetic/must-not-load-librime.dylib",
        "memory_enabled": true, "privacy_learning_enabled": true,
        "memory_db_path": run.appendingPathComponent("memory.db").path,
      ]
      func save() throws { try JSONSerialization.data(withJSONObject: document).write(to: settings) }
      func create() throws -> UnsafeMutableRawPointer {
        guard let session = settings.path.withCString({ newSession($0) }) else { throw ProbeFailure(message: "static CAPI session failed") }
        return session
      }
      try save()
      let first = try create()
      let second = try create()
      freeSession(first)
      try input(second, "zhongguo")
      let learned = try "静态词库".withCString { text in
        try "com.example.InputiaSyntheticProbe".withCString { bundle in try json(learn(second, 1, text, bundle)) }
      }
      try require(learned["decision"] as? String == "learn", "synthetic explicit learning failed")
      freeSession(second)
      let reopened = try create()
      let words = try json(hotwords(reopened, 20))["hotwords"] as? [String] ?? []
      try require(words.contains("静态词库"), "learned term missing after free/reopen")
      freeSession(reopened)
      document["schema_id"] = "double_pinyin_flypy"
      try save()
      let doublePinyin = try create()
      try input(doublePinyin, "vsgo")
      freeSession(doublePinyin)
      try verifyImages()
      print("rust_capi_static_probe=pass multi_session=true free_reopen=true pinyin_commit=true double_pinyin_commit=true learned_term_persisted=true external_librime_loaded=false synthetic_user_dir=\(user.path)")
    } catch {
      fputs("rust_capi_static_probe=failed reason=\(error)\n", stderr)
      exit(1)
    }
  }
}
