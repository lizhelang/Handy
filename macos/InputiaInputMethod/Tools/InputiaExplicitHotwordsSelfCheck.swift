import Foundation
import Darwin

@main struct InputiaExplicitHotwordsSelfCheck {
  static func main() throws {
    let words = ["lll@example.com", "compute服务器", "AEA", "额度", "南信大"]
    let store = InputiaExplicitHotwords()
    store.publish(words)
    let snapshot = store.snapshot()
    let client = NSObject()
    let context = InputiaExplicitSelectionContext(client: ObjectIdentifier(client), activation: 1,
      mode: "Chinese", code: "lll", naturalDoublePinyin: true, selection: NSRange(location: 0, length: 0), generation: snapshot.generation)
    // 模拟每个按键使语音快照失效；本地manual仍可显示和接受Space/Tab意图。
    for _ in ["l", "ll", "lll", "Space", "Tab"] {
      InputiaSharedTermsMemory.shared.clear()
      precondition(InputiaHotwordPrefix.candidates(store.snapshot().words, code: "lll") == [words[0]])
      precondition(context.admits(current: context, word: words[0], snapshot: store.snapshot(), secure: false, active: true))
    }
    precondition(InputiaHotwordPrefix.candidates(words, code: "llll").isEmpty)
    precondition(InputiaHotwordPrefix.candidates(words, code: String("llll".dropLast())) == [words[0]])
    precondition(InputiaHotwordPrefix.candidates(words, code: "com") == [words[1]])
    precondition(InputiaHotwordPrefix.candidates(words, code: "aea") == [words[2]])
    precondition(InputiaHotwordPrefix.candidates(words, code: "edu") == [words[3]])
    precondition(InputiaHotwordPrefix.candidates(words, code: "eedu") == [words[3]])
    precondition(InputiaHotwordPrefix.candidates(words, code: "nanxin") == [words[4]])
    precondition(InputiaHotwordPrefix.candidates(words, code: "njxn") == [words[4]])
    precondition(InputiaHotwordPrefix.candidates(words, code: "nanxinda").isEmpty)
    precondition(!context.admits(current: context, word: words[0], snapshot: snapshot, secure: true, active: true))
    precondition(!context.admits(current: context, word: words[0], snapshot: snapshot, secure: false, active: false))
    let moved = InputiaExplicitSelectionContext(client: ObjectIdentifier(client), activation: 1,
      mode: "Chinese", code: "lll", naturalDoublePinyin: true, selection: NSRange(location: 1, length: 0), generation: snapshot.generation)
    precondition(!context.admits(current: moved, word: words[0], snapshot: snapshot, secure: false, active: true))
    store.publish([words[1]])
    precondition(!store.admits(snapshot, word: words[0]))
    precondition(!context.admits(current: context, word: words[0], snapshot: store.snapshot(), secure: false, active: true))
    store.publish(words)
    precondition(!store.admits(snapshot, word: words[0])) // 删除后加回也不能重放旧代。
    let fixture: [String: Any] = ["settings": ["custom_words": words, "unrelated_secret": "ignored"], "history": ["not-read"]]
    let data = try JSONSerialization.data(withJSONObject: fixture)
    precondition(InputiaExplicitHotwords.decode(data) == words)
    precondition(InputiaExplicitHotwords.decode(Data("bad".utf8)).isEmpty)
    precondition(InputiaExplicitHotwords.normalize(["bad\nterm", "<|token|>", String(repeating: "x", count: 129)]).isEmpty)
    let directory = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".inputia-explicit-selfcheck-" + UUID().uuidString)
    try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    defer { try? FileManager.default.removeItem(at: directory) }
    let file = directory.appendingPathComponent("settings_store.json")
    try data.write(to: file)
    precondition(InputiaExplicitHotwords.read(file: file) == words)
    let symlink = directory.appendingPathComponent("linked.json")
    try FileManager.default.createSymbolicLink(atPath: symlink.path, withDestinationPath: file.path)
    precondition(InputiaExplicitHotwords.read(file: symlink).isEmpty)
    let hardlink = directory.appendingPathComponent("hard.json")
    precondition(link(file.path, hardlink.path) == 0)
    precondition(InputiaExplicitHotwords.read(file: file).isEmpty)
    try FileManager.default.removeItem(at: hardlink)
    let parentLink = directory.appendingPathComponent("parent")
    try FileManager.default.createSymbolicLink(atPath: parentLink.path, withDestinationPath: directory.path)
    precondition(InputiaExplicitHotwords.read(file: parentLink.appendingPathComponent("settings_store.json")).isEmpty)
    let oversized = directory.appendingPathComponent("large.json")
    precondition(FileManager.default.createFile(atPath: oversized.path, contents: nil))
    let fd = open(oversized.path, O_WRONLY)
    precondition(fd >= 0 && ftruncate(fd, 4 * 1024 * 1024 + 1) == 0); close(fd)
    precondition(InputiaExplicitHotwords.read(file: oversized).isEmpty)
    store.publish(InputiaExplicitHotwords.read(file: directory.appendingPathComponent("missing.json")))
    precondition(store.snapshot().words.isEmpty)
    print("explicit_hotwords_cache_prefix_selection=pass voice_snapshot_required=false synthetic=true native_imk_commit_tested=false")
  }
}
