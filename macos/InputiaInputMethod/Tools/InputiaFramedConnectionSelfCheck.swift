import Darwin
import Foundation

@main
struct InputiaFramedConnectionSelfCheck {
  static func pair() throws -> [Int32] {
    var descriptors: [Int32] = [0, 0]
    guard socketpair(AF_UNIX, SOCK_STREAM, 0, &descriptors) == 0 else { throw InputiaConnectionError.io }
    return descriptors
  }
  static func require(_ value: Bool) { if !value { fatalError("synthetic connection assertion failed") } }
  static func main() {
    let finished = DispatchSemaphore(value: 0)
    DispatchQueue.global().async {
      do {
        var checks = 0
        // 使用真实命名套接字保护生产connect路径；socketpair不会经过路径校验。
        let directory = "/private/tmp/inputia-connection-\(UUID().uuidString)"
        try FileManager.default.createDirectory(atPath: directory, withIntermediateDirectories: false, attributes: [.posixPermissions: 0o700])
        defer { try? FileManager.default.removeItem(atPath: directory) }
        let path = directory + "/voice.sock"
        let listener = socket(AF_UNIX, SOCK_STREAM, 0)
        require(listener >= 0)
        defer { Darwin.close(listener) }
        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)
        address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let bytes = Array(path.utf8CString)
        withUnsafeMutableBytes(of: &address.sun_path) { buffer in bytes.withUnsafeBytes { buffer.copyBytes(from: $0) } }
        let bound = withUnsafePointer(to: &address) { pointer in
          pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        require(bound == 0 && chmod(path, 0o600) == 0 && listen(listener, 1) == 0)
        let named = try InputiaFramedConnection.connect(path: path)
        let accepted = accept(listener, nil, nil)
        require(accepted >= 0)
        named.close(); Darwin.close(accepted); checks += 1
        let alias = directory + "/alias.sock"
        try FileManager.default.createSymbolicLink(atPath: alias, withDestinationPath: path)
        do { _ = try InputiaFramedConnection.connect(path: alias); fatalError("symlink accepted") }
        catch InputiaConnectionError.endpoint { checks += 1 }
        require(chmod(directory, 0o755) == 0)
        do { _ = try InputiaFramedConnection.connect(path: path); fatalError("public directory accepted") }
        catch InputiaConnectionError.endpoint { checks += 1 }
        require(chmod(directory, 0o700) == 0)
        let descriptors = try pair()
        let left = try InputiaFramedConnection.fixture(descriptor: descriptors[0])
        let right = try InputiaFramedConnection.fixture(descriptor: descriptors[1])
        for index in 0..<100 {
          try left.write(["id": String(index), "text": "合成中文🙂"])
          let received = try right.read([String: String].self)
          require(received == ["id": String(index), "text": "合成中文🙂"])
        }
        checks += 1
        left.close(); right.close()
        do { try left.write(["closed": true]); fatalError("closed stream accepted") } catch { checks += 1 }
        for header: [UInt8] in [[0,0,0,0], [0,4,0,1], [255,255,255,255]] {
          let descriptors = try pair()
          let connection = try InputiaFramedConnection.fixture(descriptor: descriptors[0])
          header.withUnsafeBytes { _ = Darwin.write(descriptors[1], $0.baseAddress, $0.count) }
          do { _ = try connection.read([String: String].self); fatalError("invalid frame accepted") }
          catch { checks += 1 }
          Darwin.close(descriptors[1])
        }
        let stalled = try pair()
        let slow = try InputiaFramedConnection.fixture(descriptor: stalled[0], timeout: 0.05)
        let partial: [UInt8] = [0,0]
        partial.withUnsafeBytes { _ = Darwin.write(stalled[1], $0.baseAddress, $0.count) }
        let begin = ProcessInfo.processInfo.systemUptime
        do { _ = try slow.read([String: String].self); fatalError("partial frame accepted") }
        catch InputiaConnectionError.timeout { checks += 1 }
        require(ProcessInfo.processInfo.systemUptime - begin < 1)
        Darwin.close(stalled[1])
        let broken = try pair()
        let writer = try InputiaFramedConnection.fixture(descriptor: broken[0])
        Darwin.close(broken[1])
        do { try writer.write(["broken": true]); fatalError("broken peer accepted") }
        catch { checks += 1 }
        let denied = try pair()
        let auth = try InputiaFramedConnection.fixture(descriptor: denied[0])
        do { try auth.authenticate { _ in throw InputiaConnectionError.endpoint }; fatalError("auth rejection ignored") }
        catch { checks += 1 }
        do { try auth.write(["forbidden": true]); fatalError("failed auth connection reused") }
        catch { checks += 1 }
        Darwin.close(denied[1])
        print("inputia_framed_connection=pass checks=\(checks) frames=100 named_socket=true main_ui_started=false")
      } catch { fatalError("synthetic connection test failed: \(error)") }
      finished.signal()
    }
    finished.wait()
    do {
      let descriptors = try pair()
      defer { Darwin.close(descriptors[1]) }
      do { _ = try InputiaFramedConnection.fixture(descriptor: descriptors[0]); fatalError("main thread allowed") }
      catch InputiaConnectionError.mainThread { print("inputia_framed_main_thread_rejected=true") }
    } catch { fatalError("main thread test failed") }
  }
}
