import Darwin
import Foundation

enum InputiaConnectionError: Error { case mainThread, endpoint, io, timeout, invalidFrame }

/// 仅供单一后台队列拥有；不在按键或AppKit主线程等待，失败后整条连接失效。
final class InputiaFramedConnection {
  static let maximumFrameBytes = 256 * 1024
  private var descriptor: Int32
  private let timeout: TimeInterval

  private init(descriptor: Int32, timeout: TimeInterval) throws {
    guard !Thread.isMainThread else { Darwin.close(descriptor); throw InputiaConnectionError.mainThread }
    self.descriptor = descriptor
    self.timeout = timeout
    var noSignal: Int32 = 1
    guard fcntl(descriptor, F_SETFL, O_NONBLOCK) == 0,
          fcntl(descriptor, F_SETFD, FD_CLOEXEC) == 0,
          setsockopt(descriptor, SOL_SOCKET, SO_NOSIGPIPE, &noSignal, socklen_t(MemoryLayout<Int32>.size)) == 0 else {
      close(); throw InputiaConnectionError.io
    }
  }

  deinit { close() }
  func close() { if descriptor >= 0 { Darwin.close(descriptor); descriptor = -1 } }

  static func connect(path: String) throws -> InputiaFramedConnection {
    guard !Thread.isMainThread else { throw InputiaConnectionError.mainThread }
    let endpoint = URL(fileURLWithPath: path)
    guard endpoint.path == path, endpoint.resolvingSymlinksInPath().path == path else { throw InputiaConnectionError.endpoint }
    var directory = stat(), node = stat()
    guard lstat(endpoint.deletingLastPathComponent().path, &directory) == 0,
          directory.st_mode & S_IFMT == S_IFDIR, directory.st_uid == geteuid(), directory.st_mode & 0o077 == 0,
          lstat(path, &node) == 0, node.st_mode & S_IFMT == S_IFSOCK,
          node.st_uid == geteuid(), node.st_mode & 0o077 == 0 else { throw InputiaConnectionError.endpoint }
    var address = sockaddr_un()
    address.sun_family = sa_family_t(AF_UNIX)
    address.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
    let bytes = Array(path.utf8CString)
    guard bytes.count <= MemoryLayout.size(ofValue: address.sun_path) else { throw InputiaConnectionError.endpoint }
    withUnsafeMutableBytes(of: &address.sun_path) { buffer in bytes.withUnsafeBytes { buffer.copyBytes(from: $0) } }
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    guard fd >= 0 else { throw InputiaConnectionError.io }
    let connection = try InputiaFramedConnection(descriptor: fd, timeout: 2)
    do {
      let result = withUnsafePointer(to: &address) { pointer in
        pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) { Darwin.connect(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
      }
      if result != 0 {
        guard errno == EINPROGRESS else { throw InputiaConnectionError.io }
        try connection.wait(events: Int16(POLLOUT), deadline: ProcessInfo.processInfo.systemUptime + 2)
        var error: Int32 = 0
        var size = socklen_t(MemoryLayout<Int32>.size)
        guard getsockopt(fd, SOL_SOCKET, SO_ERROR, &error, &size) == 0, error == 0 else { throw InputiaConnectionError.io }
      }
      var uid: uid_t = 0, gid: gid_t = 0
      guard getpeereid(fd, &uid, &gid) == 0, uid == geteuid() else { throw InputiaConnectionError.endpoint }
      return connection
    } catch { connection.close(); throw error }
  }

  /// 同UID仅为基础检查；调用者必须在读取任何握手/业务帧前用该fd验证对端签名。
  func authenticate<T>(_ body: (Int32) throws -> T) throws -> T {
    guard !Thread.isMainThread else { throw InputiaConnectionError.mainThread }
    guard descriptor >= 0 else { throw InputiaConnectionError.io }
    do { return try body(descriptor) } catch { close(); throw error }
  }

  private func wait(events: Int16, deadline: TimeInterval) throws {
    while true {
      let remaining = deadline - ProcessInfo.processInfo.systemUptime
      guard remaining > 0 else { throw InputiaConnectionError.timeout }
      var descriptor = pollfd(fd: self.descriptor, events: events, revents: 0)
      let result = poll(&descriptor, 1, Int32(min(ceil(remaining * 1000), Double(Int32.max))))
      if result > 0 { return }
      if result == 0 { throw InputiaConnectionError.timeout }
      if errno != EINTR { throw InputiaConnectionError.io }
    }
  }

  private func transfer(_ data: inout Data, writing: Bool, deadline: TimeInterval) throws {
    var offset = 0
    while offset < data.count {
      try wait(events: Int16(writing ? POLLOUT : POLLIN), deadline: deadline)
      let count = data.withUnsafeMutableBytes { buffer -> Int in
        let address = buffer.baseAddress!.advanced(by: offset)
        return writing ? Darwin.write(descriptor, address, buffer.count - offset) : Darwin.read(descriptor, address, buffer.count - offset)
      }
      if count > 0 { offset += count; continue }
      if count < 0 && (errno == EINTR || errno == EAGAIN || errno == EWOULDBLOCK) { continue }
      throw InputiaConnectionError.io
    }
  }

  func read<Value: Decodable>(_ type: Value.Type) throws -> Value {
    guard !Thread.isMainThread else { throw InputiaConnectionError.mainThread }
    guard descriptor >= 0 else { throw InputiaConnectionError.io }
    do {
      let deadline = ProcessInfo.processInfo.systemUptime + timeout
      var header = Data(count: 4)
      try transfer(&header, writing: false, deadline: deadline)
      let size = header.reduce(0) { ($0 << 8) | Int($1) }
      guard size > 0, size <= Self.maximumFrameBytes else { throw InputiaConnectionError.invalidFrame }
      var payload = Data(count: size)
      try transfer(&payload, writing: false, deadline: deadline)
      return try JSONDecoder().decode(type, from: payload)
    } catch { close(); throw error }
  }

  func write<Value: Encodable>(_ value: Value) throws {
    guard !Thread.isMainThread else { throw InputiaConnectionError.mainThread }
    guard descriptor >= 0 else { throw InputiaConnectionError.io }
    do {
      var payload = try JSONEncoder().encode(value)
      guard !payload.isEmpty, payload.count <= Self.maximumFrameBytes else { throw InputiaConnectionError.invalidFrame }
      var size = UInt32(payload.count).bigEndian
      var header = withUnsafeBytes(of: &size) { Data($0) }
      let deadline = ProcessInfo.processInfo.systemUptime + timeout
      try transfer(&header, writing: true, deadline: deadline)
      try transfer(&payload, writing: true, deadline: deadline)
    } catch { close(); throw error }
  }

  #if INPUTIA_CONNECTION_SELF_CHECK
  static func fixture(descriptor: Int32, timeout: TimeInterval = 0.1) throws -> InputiaFramedConnection {
    try InputiaFramedConnection(descriptor: descriptor, timeout: timeout)
  }
  #endif
}
