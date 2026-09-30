import Darwin
import Foundation
import Security

private var assertions = 0
private func check(_ value: Bool, _ label: String) {
  precondition(value, label)
  assertions += 1
}
private func source(_ id: String, target: Bool = false, apple: Bool = false, enabled: Bool = true)
  -> InputSourceDescriptor
{
  .init(id: id, token: id, target: target, verifiedApple: apple, enabled: enabled, selectable: true)
}
private let ime = source("com.inputia.fixture.Hans", target: true)
private let apple = source("com.apple.keylayout.ABC", apple: true)
private let other = source("com.fixture.user_choice")
private final class Fixture: InputSourceBackend {
  var selected = ime
  var fallbackSource = apple
  var originalSource = ime
  var changes: UInt64 = 0
  var clock: UInt64 = 100
  var actions = 0
  var errorAfterSelect = false
  var readbackMismatch = false
  var duringSelect: (() -> Void)?
  var afterSelect: (() -> Void)?
  var lookupOriginal: (() -> Void)?
  var snapshotError = false
  func current() throws -> InputSourceDescriptor {
    if snapshotError { throw InstallCodeError.rejected("fixture_read_failed") }
    return selected
  }
  func generation() -> UInt64 { changes }
  func now() -> UInt64 { clock }
  func fallback() throws -> InputSourceDescriptor { fallbackSource }
  func original(_ id: String) throws -> InputSourceDescriptor {
    lookupOriginal?()
    return originalSource
  }
  func select(
    _ source: InputSourceDescriptor, expecting: InputSourceDescriptor, generation: UInt64,
    authorize: () -> Bool
  ) throws {
    duringSelect?()
    guard selected == expecting, changes == generation, authorize() else {
      throw InstallCodeError.rejected("input_source_effect_revoked")
    }
    actions += 1
    selected = readbackMismatch ? other : source
    afterSelect?()
    if errorAfterSelect { throw InstallCodeError.rejected("input_source_select_failed") }
  }
  func userSelect(_ source: InputSourceDescriptor) {
    selected = source
    changes += 1
  }
}
private func withMachine(_ work: (Fixture, InputSourceMachine) throws -> Void) throws {
  let backend = Fixture()
  let machine = try InputSourceMachine(
    backend: backend, deadline: 1000, slot: InputSourceSlot.acquire())
  try work(backend, machine)
}
private func checkRejected(_ label: String, _ work: () throws -> Void) {
  do {
    try work()
    preconditionFailure(label)
  } catch { assertions += 1 }
}
private func checkSecuredContract() throws {
  let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
  try FileManager.default.createDirectory(
    at: root.appendingPathComponent("Contents"), withIntermediateDirectories: true)
  defer { try? FileManager.default.removeItem(at: root) }
  let request = InstallCodeRequest(
    schema_version: 1,
    subject: .init(
      transaction_id: "11111111-1111-4111-8111-111111111111",
      plan_sha256: String(repeating: "a", count: 64),
      installation_id: "22222222-2222-4222-8222-222222222222", new_release_id: "inputia-new"),
    purpose: "previous_release", product_id: "com.inputia", role: "ime",
    exact_bundle_path: root.path,
    bundle_id: "com.inputia.fixture", release_id: "inputia-old", version: "1.0.0", build: 1,
    source_commit: String(repeating: "b", count: 40), team_id: "TESTTEAM01",
    architectures: ["arm64"],
    cdhashes: [String(repeating: "c", count: 40)])
  let modeID = request.bundle_id + ".Hans"
  func plist(_ modes: [String: [String: Any]]) -> [String: Any] {
    [
      "CFBundleIdentifier": request.bundle_id, "TISInputSourceID": request.bundle_id,
      "ComponentInputModeDict": ["tsInputModeListKey": modes],
    ]
  }
  let secured = plist([
    modeID: ["TISInputSourceID": modeID, "tsInputModeMenuIconFileKey": "icon.png"]
  ])
  var captured = SecuredInstallPlist()
  try captured.include([kSecCodeInfoPList as String: secured])
  try captured.include([kSecCodeInfoPList as String: secured])
  check(captured.value != nil, "same secured dictionary across architecture slices")
  // 验过的字典不随随后路径读取改变；超预算的磁盘文件也不会被合同解析器读取。
  try Data(repeating: 120, count: 1_048_577).write(
    to: root.appendingPathComponent("Contents/Info.plist"))
  let contract = try SourceContract.fromSecuredPlist(captured.value!, request: request)
  check(
    contract.icons[modeID]?.lastPathComponent == "icon.png",
    "contract bound to captured secured bytes, not replaced path")
  checkRejected("different architecture secured plist accepted") {
    try captured.include([
      kSecCodeInfoPList as String: plist([
        modeID: ["TISInputSourceID": modeID, "tsInputModeMenuIconFileKey": "other.png"]
      ])
    ])
  }
  checkRejected("missing secured plist accepted") { try captured.include([:]) }
  checkRejected("unbounded icon accepted") {
    _ = try SourceContract.fromSecuredPlist(
      plist([
        modeID: [
          "TISInputSourceID": modeID,
          "tsInputModeMenuIconFileKey": String(repeating: "x", count: 257),
        ]
      ]), request: request)
  }
  checkRejected("path traversal accepted") {
    _ = try SourceContract.fromSecuredPlist(
      plist([modeID: ["TISInputSourceID": modeID, "tsInputModeMenuIconFileKey": "../icon.png"]]),
      request: request)
  }
  checkRejected("unbounded mode list accepted") {
    let modes = Dictionary(
      uniqueKeysWithValues: (0..<33).map { index in
        let id = request.bundle_id + ".Mode\(index)"
        return (
          id, ["TISInputSourceID": id, "tsInputModeMenuIconFileKey": "icon.png"] as [String: Any]
        )
      })
    _ = try SourceContract.fromSecuredPlist(plist(modes), request: request)
  }
  var delivered = false
  checkRejected("unverified bundle delivered secured contract") {
    _ = try verifyInstallCode(request) { _ in delivered = true }
  }
  check(!delivered, "verification failure never calls capture callback")
}
@main
struct InputSourceSelfCheck {
  static func main() throws {
    try checkSecuredContract()
    // 没有CarbonInputSourceBackend构造、实际源枚举或任何系统TISSelect调用。
    try withMachine { backend, machine in
      check(machine.observation.state == "prepared" && backend.actions == 0, "prepare is read only")
      check(machine.detach(authorize: { true }).state == "detached_observed", "fallback readback")
      check(
        machine.detach(authorize: { true }).state == "detached_observed" && backend.actions == 1,
        "detach retry never replays")
      check(
        machine.assertDetached(authorize: { true }).state == "detached_observed", "fresh assertion")
      check(
        machine.restore(authorize: { true }).state == "restored_observed",
        "conditional restore readback")
      check(
        machine.restore(authorize: { true }).state == "restored_observed" && backend.actions == 2,
        "restore retry does not replay")
      check(!machine.observation.ownershipExact, "no claim of CAS ownership")
    }
    do {
      let backend = Fixture()
      backend.selected = other
      let machine = try InputSourceMachine(
        backend: backend, deadline: 1000, slot: InputSourceSlot.acquire())
      check(
        machine.detach(authorize: { true }).state == "already_detached",
        "non-target remains selected")
      _ = machine.restore(authorize: { true })
      check(
        backend.actions == 0 && backend.selected == other, "no restore authority without own change"
      )
    }
    try withMachine { backend, machine in
      _ = machine.detach(authorize: { true })
      backend.userSelect(other)
      check(
        machine.restore(authorize: { true }).state == "preserved_user_selection",
        "preserve user change")
      check(
        backend.actions == 1 && backend.selected == other, "no overwrite of observed new source")
    }
    try withMachine { backend, machine in
      _ = machine.detach(authorize: { true })
      backend.userSelect(other)
      backend.userSelect(apple)
      check(
        machine.restore(authorize: { true }).reason == "selection_history_unknown",
        "ABA detected despite same final ID")
      check(backend.actions == 1, "ABA never restores")
    }
    try withMachine { backend, machine in
      _ = machine.detach(authorize: { true })
      backend.changes += 1
      check(
        machine.restore(authorize: { true }).state == "uncertain",
        "late own notification not claimed as own")
      check(backend.actions == 1, "late notification forbids restore")
    }
    try withMachine { backend, machine in
      backend.afterSelect = { backend.changes += 1 }
      check(
        machine.detach(authorize: { true }).reason == "selection_history_unknown",
        "event during select not swallowed")
      _ = machine.restore(authorize: { true })
      check(backend.actions == 1, "uncertain own event not restored")
    }
    for fallback in [
      source("com.apple.keylayout.ABC"),
      source("com.apple.keylayout.ABC", apple: true, enabled: false),
    ] {
      try withMachine { backend, machine in
        backend.fallbackSource = fallback
        check(
          machine.detach(authorize: { true }).reason == "no_verified_fallback",
          "spoofed ID or disabled source rejected")
        check(backend.actions == 0, "no fallback enable or select")
      }
    }
    try withMachine { backend, machine in
      backend.errorAfterSelect = true
      check(machine.detach(authorize: { true }).state == "uncertain", "unknown syscall outcome")
      _ = machine.detach(authorize: { true })
      _ = machine.restore(authorize: { true })
      check(backend.actions == 1, "unknown outcome cannot replay or restore")
    }
    try withMachine { backend, machine in
      backend.readbackMismatch = true
      check(
        machine.detach(authorize: { true }).reason == "detach_result_uncertain",
        "success status without expected readback")
      check(backend.actions == 1, "no automatic second route")
    }
    try withMachine { backend, machine in
      backend.duringSelect = { backend.clock = 1001 }
      check(
        machine.detach(authorize: { true }).state == "uncertain" && backend.actions == 0,
        "slow pre-effect validation expires")
    }
    try withMachine { backend, machine in
      backend.afterSelect = { backend.clock = 1001 }
      check(
        machine.detach(authorize: { true }).reason == "detach_result_uncertain",
        "slow select cannot report stale success")
    }
    try withMachine { backend, machine in
      var valid = true
      backend.duringSelect = { valid = false }
      _ = machine.detach(authorize: { valid })
      check(backend.actions == 0, "last marker callback before select")
    }
    try withMachine { backend, machine in
      var valid = true
      backend.afterSelect = { valid = false }
      check(
        machine.detach(authorize: { valid }).state == "uncertain",
        "revocation during syscall visible")
    }
    try withMachine { backend, machine in
      _ = machine.detach(authorize: { true })
      backend.lookupOriginal = { backend.userSelect(other) }
      _ = machine.restore(authorize: { true })
      check(
        backend.actions == 1 && backend.selected == other,
        "slow original verification cannot overwrite new choice")
    }
    try withMachine { backend, machine in
      _ = machine.detach(authorize: { true })
      backend.clock = 1001
      check(
        machine.assertDetached(authorize: { true }).state == "uncertain",
        "expired proof not returned")
    }
    try withMachine { backend, machine in
      _ = machine.detach(authorize: { true })
      backend.originalSource = source("com.inputia.fixture.Hans", target: true, enabled: false)
      check(
        machine.restore(authorize: { true }).reason == "original_source_unavailable",
        "removed original not enabled")
      check(backend.actions == 1, "disabled original remains untouched")
    }
    do {
      let slot = try InputSourceSlot.acquire()
      do {
        _ = try InputSourceSlot.acquire()
        preconditionFailure("overlap accepted")
      } catch InstallCodeError.rejected { assertions += 1 }
      withExtendedLifetime(slot) {}
    }
    do {
      _ = try InputSourceSlot.acquire()
      assertions += 1
    }
    do {
      let backend = Fixture()
      do {
        let machine = try InputSourceMachine(
          backend: backend, deadline: 1000, slot: InputSourceSlot.acquire())
        _ = machine.detach(authorize: { true })
      }
      check(backend.actions == 1 && backend.selected == apple, "Drop never chooses source")
    }
    try withMachine { backend, machine in
      _ = machine.detach(authorize: { true })
      backend.userSelect(other)
      check(
        machine.detach(authorize: { true }).state == "preserved_user_selection"
          && backend.actions == 1,
        "detach retry reobserves without replaying a stale success")
    }
    try withMachine { backend, machine in
      _ = machine.detach(authorize: { true })
      _ = machine.restore(authorize: { true })
      backend.userSelect(other)
      check(
        machine.restore(authorize: { true }).state == "preserved_user_selection"
          && backend.actions == 2,
        "restore retry does not return stale restored observation")
    }
    var output: UnsafeMutableRawPointer?
    let reply = iuisInputSourcePrepare(nil, 1, &output)!
    check(
      output == nil && String(cString: reply).contains("invalid_request"),
      "invalid C request fails before system backend")
    iuisStringFree(reply)
    print(
      "InputSourceSelfCheck: \(assertions) assertions passed; injected backend only; actual TIS NOT_RUN"
    )
  }
}
