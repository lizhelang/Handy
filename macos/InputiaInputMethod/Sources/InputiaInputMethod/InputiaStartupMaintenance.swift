import Foundation
import Darwin

@_silgen_name("inputia_maintenance_startup_check")
private func maintenanceStartupCheck() -> UnsafeMutablePointer<CChar>?
@_silgen_name("inputia_string_free")
private func maintenanceStringFree(_ pointer: UnsafeMutablePointer<CChar>?)

/// 共享 Rust 解析器在任何 profile/session/IMK/设置启动前检查，v1 与 v2 使用同一路径。
enum InputiaStartupMaintenance {
  static func requireNormalStart() {
    guard let pointer = maintenanceStartupCheck() else { exit(78) }
    defer { maintenanceStringFree(pointer) }
    struct Reply: Decodable { let ok: Bool; let normal_start_allowed: Bool }
    guard let reply = try? JSONDecoder().decode(Reply.self, from: Data(String(cString: pointer).utf8)),
      reply.ok, reply.normal_start_allowed else {
      NSLog("Inputia 正在维护或维护记录需要修复，普通启动已暂停")
      exit(78)
    }
  }
}
