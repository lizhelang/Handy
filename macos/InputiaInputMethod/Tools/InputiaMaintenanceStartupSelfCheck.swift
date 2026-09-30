import Foundation
import Darwin

// 合成 ABI 回复只验证 Swift 启动分支；不访问真实维护目录，也不冒充 Rust/系统验证。
@_cdecl("inputia_maintenance_startup_check")
func syntheticMaintenanceCheck() -> UnsafeMutablePointer<CChar>? {
  let scenario = CommandLine.arguments.last ?? ""
  if scenario == "null" { return nil }
  let replies = [
    "clear": #"{"ok":true,"normal_start_allowed":true}"#,
    "active": #"{"ok":false,"normal_start_allowed":false,"code":"maintenance_active"}"#,
    "corrupt": #"{"ok":false,"normal_start_allowed":false,"code":"invalid_marker"}"#,
    "wrong-result": #"{"ok":false,"normal_start_allowed":true}"#,
    "missing-field": #"{"ok":true}"#,
    "bad-json": "{",
  ]
  return strdup(replies[scenario] ?? "")
}

@_cdecl("inputia_string_free")
func syntheticStringFree(_ pointer: UnsafeMutablePointer<CChar>?) { free(pointer) }

@main
struct InputiaMaintenanceStartupSelfCheck {
  static func main() throws {
    if CommandLine.arguments.count == 2 {
      InputiaStartupMaintenance.requireNormalStart()
      print("synthetic_writer_reached")
      return
    }
    for scenario in ["clear", "active", "corrupt", "wrong-result", "missing-field", "bad-json", "null"] {
      let task = Process()
      let output = Pipe()
      task.executableURL = URL(fileURLWithPath: CommandLine.arguments[0])
      task.arguments = [scenario]
      task.standardOutput = output
      task.standardError = Pipe()
      try task.run()
      task.waitUntilExit()
      let text = String(data: output.fileHandleForReading.readDataToEndOfFile(), encoding: .utf8) ?? ""
      precondition(task.terminationStatus == (scenario == "clear" ? 0 : 78), scenario)
      precondition(text.contains("synthetic_writer_reached") == (scenario == "clear"), scenario)
    }
    print("maintenance_startup_synthetic_check=pass cases=7 real_user_data_accessed=false")
  }
}
