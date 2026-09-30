import Darwin
import Foundation

// 本可执行文件仅链接合成 ABI；不链接 CAPI、不访问用户设置、不开输入会话。
private let storeID = "11111111-1111-4111-8111-111111111111"
private var values: [String: Any] = ["memory_enabled":true,"chinese_script":"simplified","candidate_font_size":14]
private var revision = "1"
private var calls: [[String: Any]] = []
private var applyMode = "saved"
private var requestedSession: [String: Any] = [:]
private var applicationFields: [String] = []
private func snapshotRaw() -> [String: Any] {
  ["store_id":storeID,"revision":revision,"values_digest":String(repeating:"a",count:64),"values":values]
}
private func encode(_ value: [String: Any]) -> UnsafeMutablePointer<CChar>? {
  strdup(String(decoding: try! JSONSerialization.data(withJSONObject:value), as:UTF8.self))
}
@_cdecl("inputia_settings_request")
func fixtureSettingsRequest(_ input: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>? {
  let request = try! JSONSerialization.jsonObject(with: Data(String(cString:input).utf8)) as! [String:Any]
  calls.append(request)
  switch request["action"] as? String {
  case "read": return encode(["ok":true,"snapshot":snapshotRaw()])
  case "apply", "import_external":
    if ["commit_uncertain","external_changed","external_edit","busy","maintenance"].contains(applyMode) { return encode(["ok":false,"code":applyMode]) }
    if applyMode == "conflict" { return encode(["ok":true,"result":["status":"conflict","current":snapshotRaw()]]) }
    let change = request["request"] as! [String:Any]
    if let patch = change["patch"] as? [String:Any] { values.merge(patch) { _, new in new } }
    revision = String(UInt64(revision)! + 1)
    return encode(["ok":true,"result":["status":"saved","commit_revision":revision,"current":snapshotRaw()]])
  case "inspect_external": return encode(["ok":true,"external":["store_id":storeID,"revision":revision,
    "observed_file_digest":String(repeating:"b",count:64),"values":values]])
  case "application_status": return encode(["ok":true,"application":["scope":"observed_engine_sessions","lease_ms":2500,
    "current_store_id":storeID,"current_revision":revision,"current_values_digest":String(repeating:"a",count:64),"sessions":[]]])
  default: return encode(["ok":false,"code":"invalid_request"])
  }
}
@_cdecl("inputia_string_free")
func fixtureStringFree(_ value: UnsafeMutablePointer<CChar>?) { free(value) }
@_cdecl("inputia_session_new_from_settings_snapshot")
func fixtureSnapshotSession(_ input: UnsafePointer<CChar>) -> UnsafeMutableRawPointer? {
  requestedSession = try! JSONSerialization.jsonObject(with:Data(String(cString:input).utf8)) as! [String:Any]
  return UnsafeMutableRawPointer(bitPattern: 1)
}
@_cdecl("inputia_session_settings_applied")
func fixtureSettingsApplied(_ session: UnsafeMutableRawPointer, _ input: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>? {
  let request = try! JSONSerialization.jsonObject(with:Data(String(cString:input).utf8)) as! [String:Any]
  applicationFields = request["applied_fields"] as! [String]
  return encode(["ok":true])
}
@_cdecl("inputia_settings_flush_applications")
func fixtureFlush(_ input: UnsafePointer<CChar>) -> UnsafeMutablePointer<CChar>? { encode(["ok":true]) }

@main
struct InputiaSettingsStoreSelfCheck {
  static func main() throws {
    var checks = 0
    func check(_ condition: Bool, _ message: String) { precondition(condition,message); checks += 1 }
    let path = "/synthetic-only/settings.json"
    let base = try InputiaSettingsStore.read(path:path)
    var invalid = snapshotRaw(); invalid["values_digest"] = String(repeating:"G",count:64)
    check((try? InputiaSettingsStore.Snapshot(invalid)) == nil,"reject nonhex digest")
    invalid = snapshotRaw(); invalid["revision"] = "01"
    check((try? InputiaSettingsStore.Snapshot(invalid)) == nil,"reject noncanonical revision")
    struct Fields: Codable { var memory_enabled: Bool; var rime_shared_data_dir: String; var unknown: String }
    let patch = try InputiaSettingsStore.dirtyPatch(from:Fields(memory_enabled:true,rime_shared_data_dir:"old",unknown:"keep"),
      to:Fields(memory_enabled:false,rime_shared_data_dir:"new",unknown:"changed"))
    check(patch.count == 1 && patch["memory_enabled"] as? Bool == false,"dirty fields exclude resources and extensions")
    let edit = InputiaSettingsEdit(base)
    applyMode = "commit_uncertain"
    do { _ = try edit.apply(path:path,patch:["memory_enabled":false]); preconditionFailure("uncertain cannot be success") }
    catch let failure as InputiaSettingsStore.Failure { check(failure.code == "commit_uncertain","uncertain surfaced") }
    let first = edit.pending!.raw
    check(calls.filter { $0["action"] as? String == "apply" }.count == 2,"bounded same operation retry")
    let attempted = calls.filter { $0["action"] as? String == "apply" }.map { $0["request"] as! [String:Any] }
    check(NSDictionary(dictionary:attempted[0]).isEqual(to:attempted[1]),"retries are byte-equivalent values")
    applyMode = "busy"
    do { _ = try edit.apply(path:path,patch:["memory_enabled":true]); preconditionFailure("busy cannot confirm uncertainty") }
    catch let failure as InputiaSettingsStore.Failure { check(failure.code == "busy","busy remains pending after uncertainty") }
    check(NSDictionary(dictionary:edit.pending!.raw).isEqual(to:first),"uncertain then busy retains exact operation")
    applyMode = "saved"
    let saved = try edit.apply(path:path,patch:["memory_enabled":true])!
    let retried = calls.last!["request"] as! [String:Any]
    check(NSDictionary(dictionary:first).isEqual(to:retried),"new UI choice cannot replace uncertain operation")
    check(saved.current.values["memory_enabled"] as? Bool == false && edit.pending == nil,"resolved exact operation")
    let stale = InputiaSettingsEdit(base)
    applyMode = "conflict"
    let beforeCalls = calls.count
    let conflict = try stale.apply(path:path,patch:["chinese_script":"traditional"])!
    check(conflict.status == "conflict" && stale.pending == nil && stale.base.identity == conflict.current.identity,"conflict adopts current and requires reselection")
    check(calls.count == beforeCalls + 1,"conflict never rebases toggle")
    let externallyChanged = InputiaSettingsEdit(stale.base)
    applyMode = "external_edit"
    do { _ = try externallyChanged.apply(path:path,patch:["memory_enabled":false]); preconditionFailure("external edit cannot save") }
    catch { check(externallyChanged.pending == nil,"first external edit rejection permits preview and import") }
    applyMode = "saved"
    let external = try InputiaSettingsStore.inspectExternal(path:path)
    let operation = InputiaSettingsStore.ImportOperation(external)
    check(operation.raw["observed_file_digest"] as? String == String(repeating:"b",count:64),"external import pins displayed bytes")
    applyMode = "external_changed"
    do { _ = try InputiaSettingsStore.importExternal(path:path,operation:operation); preconditionFailure("changed external cannot import") }
    catch let error as InputiaSettingsStore.Failure { check(error.code == "external_changed","external change requires new preview") }
    var reads = 0, failed = false
    let cache = InputiaSettingsCache(path:path,polling:false) { _ in
      reads += 1
      if failed { throw InputiaSettingsStore.Failure(code:"external_edit") }
      return try InputiaSettingsStore.Snapshot(snapshotRaw())
    }
    for _ in 0..<10_000 { _ = cache.state.snapshot }
    check(reads == 1,"key-path cache reads perform no file or ABI IO")
    cache.publish(base)
    check(cache.state.snapshot!.revision == revision,"late old poll cannot regress revision")
    failed = true; cache.refresh()
    check(cache.state.snapshot == nil && cache.state.failure == "external_edit","bad external file never becomes defaults")
    failed = false; cache.refresh()
    check(cache.state.snapshot != nil,"explicit successful reread recovers cache")
    var uptime: TimeInterval = 1
    let retry = InputiaSettingsRetryGate(uptime: { uptime })
    check(retry.begin(identity:base.identity,generation:1),"first session initialization attempted")
    check(!retry.begin(identity:base.identity,generation:1),"same read never retries in key path")
    check(!retry.begin(identity:base.identity,generation:2),"fresh read still honors monotonic backoff")
    uptime = 3
    check(retry.begin(identity:base.identity,generation:2),"same revision recovers after fresh read and backoff")
    let oldSession = UnsafeMutableRawPointer(bitPattern:11)!, newSession = UnsafeMutableRawPointer(bitPattern:22)!
    var held: UnsafeMutableRawPointer? = oldSession
    var released: [UnsafeMutableRawPointer?] = []
    check(!InputiaSettingsSessionSwap.replace(&held,open:{nil},release:{released.append($0)})
      && held == oldSession && released.isEmpty,"failed new session preserves the only working session")
    check(InputiaSettingsSessionSwap.replace(&held,open:{
      check(released.isEmpty,"old session remains live throughout construction")
      return newSession
    },release:{released.append($0)}) && held == newSession && released == [oldSession],"successful replacement frees old session once")
    var unsafe = snapshotRaw(); unsafe["values"] = ["rime_user_data_dir":"/daily/rime","memory_db_path":"/daily/memory.db"]
    let unsafeSnapshot = try InputiaSettingsStore.Snapshot(unsafe)
    check((try? InputiaSettingsStore.validateRuntimePaths(unsafeSnapshot,
      expected:["rime_user_data_dir":"/candidate/rime","memory_db_path":"/candidate/inputia_memory.db"],
      required:["rime_user_data_dir","memory_db_path"])) == nil,"copied daily settings cannot open candidate data")
    try InputiaSettingsStore.validateRuntimePaths(unsafeSnapshot,
      expected:["rime_user_data_dir":"/daily/rime","memory_db_path":"/daily/memory.db"],required:["rime_user_data_dir","memory_db_path"])
    checks += 1
    let current = cache.state.snapshot!
    _ = InputiaSettingsStore.openSession(path:path,snapshot:current,sharedData:"/synthetic/Resources",withoutMemory:true)
    check(requestedSession["store_id"] as? String == current.storeID && requestedSession["revision"] as? String == current.revision
      && requestedSession["values_digest"] as? String == current.digest,"session pins exact snapshot")
    check(requestedSession["without_memory"] as? Bool == true && requestedSession["rime_shared_data_dir"] as? String == "/synthetic/Resources","resource override is request only")
    check(InputiaSettingsStore.reportApplication(session:UnsafeMutableRawPointer(bitPattern:1)!,applied:["script_toggle_shortcut"])
      && applicationFields == ["script_toggle_shortcut"],"native receipt claims only actual field")
    let empty = try InputiaSettingsStore.applicationStatus(path:path)
    check(empty.summary(for:current).contains("等待"),"no observed sessions never means applied")
    let session: [String:Any] = ["instance_id":UUID().uuidString,"store_id":current.storeID,"revision":current.revision,
      "values_digest":current.digest,"applied_fields":["script_toggle_shortcut"],"unavailable_fields":["memory_enabled"]]
    let observation = try InputiaSettingsStore.ApplicationStatus(["scope":"observed_engine_sessions","lease_ms":2500,
      "current_store_id":current.storeID,"current_revision":current.revision,"current_values_digest":current.digest,"sessions":[session]])
    check(observation.summary(for:current).contains("1 个输入会话") && observation.summary(for:current).contains("降级")
      && observation.summary(for:current).contains("字体待确认"),"partial/degraded observed confirmation is explicit")
    print("settings_store_swift_checks=\(checks) user_settings_touched=false native_engine_acceptance=NOT_RUN")
  }
}
