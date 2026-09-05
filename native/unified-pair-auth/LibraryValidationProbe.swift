import Darwin
import Foundation

/// 仅验证给定库能否加载，不创建Rime session、不读写用户词典。
@main
struct LibraryValidationProbe {
  static func main() {
    guard CommandLine.arguments.count == 2 else {
      fputs("usage: LibraryValidationProbe ABSOLUTE_LIBRARY\n", stderr)
      exit(2)
    }
    let path = CommandLine.arguments[1]
    guard path.hasPrefix("/") else { exit(2) }
    guard let handle = dlopen(path, RTLD_NOW | RTLD_LOCAL) else {
      let reason = dlerror().map { String(cString: $0) } ?? "unknown"
      print("library_load=denied reason=\(reason) rime_session_created=false")
      exit(1)
    }
    defer { dlclose(handle) }
    print("library_load=allowed rime_api_present=\(dlsym(handle, "rime_get_api") != nil) rime_session_created=false")
  }
}
