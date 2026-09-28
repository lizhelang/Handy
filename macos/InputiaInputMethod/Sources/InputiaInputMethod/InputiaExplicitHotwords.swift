import Foundation
import Darwin

/// 基础输入词典只读取用户显式设置；不依赖语音目标、AX字段、历史或学习库。
final class InputiaExplicitHotwords {
  struct Snapshot: Equatable {
    let generation: UInt64
    let words: [String]
  }
  static let shared = InputiaExplicitHotwords()
  private let lock = NSLock()
  private let queue = DispatchQueue(label: "Inputia.explicit-hotwords")
  private var value = Snapshot(generation: 0, words: [])
  private var timer: DispatchSourceTimer?
  var didChange: (() -> Void)?

  func snapshot() -> Snapshot {
    lock.lock(); defer { lock.unlock() }
    return value
  }
  func admits(_ snapshot: Snapshot, word: String) -> Bool {
    lock.lock(); defer { lock.unlock() }
    return value == snapshot && value.words.contains(word)
  }
  func start(file: URL) {
    queue.async { [weak self] in
      guard let self, self.timer == nil else { return }
      let timer = DispatchSource.makeTimerSource(queue: self.queue)
      timer.schedule(deadline: .now(), repeating: .milliseconds(500))
      timer.setEventHandler { [weak self] in
        guard let self else { return }
        // 只在后台IO；文件失败、格式错误均清空，绝不保留失效词汇。
        let words = Self.read(file: file)
        self.publish(words)
      }
      self.timer = timer
      timer.resume()
    }
  }
  func publish(_ words: [String]) {
    lock.lock()
    let changed = value.words != words
    if changed { value = Snapshot(generation: value.generation &+ 1, words: words) }
    lock.unlock()
    if changed { DispatchQueue.main.async { [weak self] in self?.didChange?() } }
  }
  static func read(file: URL) -> [String] {
    // 逐层固定目录描述符，拒绝叶节点及任何父路径符号链接；不扫描词库目录。
    let parts = file.standardizedFileURL.pathComponents.filter { $0 != "/" }
    guard !parts.isEmpty else { return [] }
    var directory = open("/", O_RDONLY | O_DIRECTORY | O_CLOEXEC)
    guard directory >= 0 else { return [] }
    defer { close(directory) }
    for part in parts.dropLast() {
      let next = openat(directory, part, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC)
      guard next >= 0 else { return [] }
      close(directory); directory = next
    }
    let descriptor = openat(directory, parts.last!, O_RDONLY | O_NOFOLLOW | O_CLOEXEC)
    guard descriptor >= 0 else { return [] }
    defer { close(descriptor) }
    var metadata = stat()
    guard fstat(descriptor, &metadata) == 0, metadata.st_mode & mode_t(S_IFMT) == mode_t(S_IFREG),
      metadata.st_uid == getuid(), metadata.st_nlink == 1, metadata.st_mode & 0o022 == 0, metadata.st_size >= 0, metadata.st_size <= 4 * 1024 * 1024 else { return [] }
    var data = Data(); var buffer = [UInt8](repeating: 0, count: 4096)
    while true {
      let count = buffer.withUnsafeMutableBytes { Darwin.read(descriptor, $0.baseAddress, $0.count) }
      guard count >= 0 else { return [] }
      if count == 0 { break }
      guard data.count + count <= 4 * 1024 * 1024 else { return [] }
      data.append(contentsOf: buffer.prefix(count))
    }
    return decode(data)
  }
  static func decode(_ data: Data) -> [String] {
    guard data.count <= 4 * 1024 * 1024,
      let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
      let settings = object["settings"] as? [String: Any], let words = settings["custom_words"] as? [String]
    else { return [] }
    return normalize(words)
  }
  static func normalize(_ words: [String]) -> [String] {
    var result: [String] = []; var bytes = 0; var seen = Set<String>()
    for raw in words {
      let word = raw.trimmingCharacters(in: .whitespacesAndNewlines)
      guard !word.isEmpty, word.count <= 128, !word.contains("<|"), !word.contains("|>"),
        !word.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
        seen.insert(word).inserted else { continue }
      guard result.count < 256, bytes + word.utf8.count <= 16 * 1024 else { break }
      bytes += word.utf8.count; result.append(word)
    }
    return result
  }
}

/// 一次本地IMK选择的输入契约；语音target失效不会改变这个基础打字合同。
struct InputiaExplicitSelectionContext: Equatable {
  let client: ObjectIdentifier
  let activation: UInt64
  let mode: String
  let code: String
  let naturalDoublePinyin: Bool
  let selection: NSRange
  let generation: UInt64
  func admits(current: Self, word: String, snapshot: InputiaExplicitHotwords.Snapshot, secure: Bool, active: Bool) -> Bool {
    !secure && active && self == current && generation == snapshot.generation && snapshot.words.contains(word)
  }
}
