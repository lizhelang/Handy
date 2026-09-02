import Foundation

struct InputiaHandyDataPaths: Equatable {
  let root: URL
  let history: URL
  let clipboard: URL

  static func defaultPaths(baseURL: URL? = nil) -> Self {
    let base = baseURL
      ?? FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first
      ?? URL(fileURLWithPath: NSTemporaryDirectory())
    let root = base.appendingPathComponent("com.pais.handy", isDirectory: true)
    return Self(
      root: root,
      history: root.appendingPathComponent("history.db"),
      clipboard: root.appendingPathComponent("clipboard.db")
    )
  }
}

protocol InputiaHandyMemoryImporting {
  func importHandyHistory(path: String, bundleId: String, limit: Int) -> Int?
  func importHandyClipboard(path: String, bundleId: String, limit: Int) -> Int?
}

extension InputiaRustBridge: InputiaHandyMemoryImporting {}

struct InputiaHandyMemorySyncResult: Equatable {
  enum SourceResult: Equatable {
    case skipped
    case missing
    case failed
    case imported(Int)

    func description(sourceName: String) -> String {
      switch self {
      case .skipped:
        return "\(sourceName) 跳过"
      case .missing:
        return "\(sourceName) 未找到"
      case .failed:
        return "\(sourceName) 失败"
      case .imported(let count):
        return "\(sourceName) \(count)"
      }
    }
  }

  let history: SourceResult
  let clipboard: SourceResult
  let rootPath: String

  var summaryText: String {
    "\(history.description(sourceName: "语音"))，\(clipboard.description(sourceName: "剪贴板"))"
  }

  var statusText: String {
    "\(summaryText)\n数据目录：\(rootPath)"
  }
}

enum InputiaHandyMemorySync {
  static let handyBundleIdentifier = "com.pais.handy"
  static let defaultLimit = 2_000

  static func sync(
    importer: InputiaHandyMemoryImporting,
    paths: InputiaHandyDataPaths = .defaultPaths(),
    includeHistory: Bool,
    includeClipboard: Bool,
    limit: Int = defaultLimit,
    fileExists: (String) -> Bool = { FileManager.default.fileExists(atPath: $0) }
  ) -> InputiaHandyMemorySyncResult {
    let history = syncHistory(
      importer: importer,
      paths: paths,
      include: includeHistory,
      limit: limit,
      fileExists: fileExists
    )
    let clipboard = syncClipboard(
      importer: importer,
      paths: paths,
      include: includeClipboard,
      limit: limit,
      fileExists: fileExists
    )
    return InputiaHandyMemorySyncResult(
      history: history,
      clipboard: clipboard,
      rootPath: paths.root.path
    )
  }

  static func statusText(
    paths: InputiaHandyDataPaths = .defaultPaths(),
    fileExists: (String) -> Bool = { FileManager.default.fileExists(atPath: $0) }
  ) -> String {
    let hasHistory = fileExists(paths.history.path)
    let hasClipboard = fileExists(paths.clipboard.path)
    if hasHistory || hasClipboard {
      return "Handy 数据：" + paths.root.path
    }
    return "未找到 Handy 数据：" + paths.root.path
  }

  private static func syncHistory(
    importer: InputiaHandyMemoryImporting,
    paths: InputiaHandyDataPaths,
    include: Bool,
    limit: Int,
    fileExists: (String) -> Bool
  ) -> InputiaHandyMemorySyncResult.SourceResult {
    guard include else {
      return .skipped
    }
    guard fileExists(paths.history.path) else {
      return .missing
    }
    guard
      let imported = importer.importHandyHistory(
        path: paths.history.path,
        bundleId: handyBundleIdentifier,
        limit: limit
      )
    else {
      return .failed
    }
    return .imported(imported)
  }

  private static func syncClipboard(
    importer: InputiaHandyMemoryImporting,
    paths: InputiaHandyDataPaths,
    include: Bool,
    limit: Int,
    fileExists: (String) -> Bool
  ) -> InputiaHandyMemorySyncResult.SourceResult {
    guard include else {
      return .skipped
    }
    guard fileExists(paths.clipboard.path) else {
      return .missing
    }
    guard
      let imported = importer.importHandyClipboard(
        path: paths.clipboard.path,
        bundleId: handyBundleIdentifier,
        limit: limit
      )
    else {
      return .failed
    }
    return .imported(imported)
  }
}
