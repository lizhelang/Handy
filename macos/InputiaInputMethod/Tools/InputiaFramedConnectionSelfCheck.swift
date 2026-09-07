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
        print("inputia_framed_connection=pass checks=\(checks) frames=100 socketpair_only=true main_ui_started=false")
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
