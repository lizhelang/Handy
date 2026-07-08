import Darwin
import Foundation

private final class MockImporter: InputiaHandyMemoryImporting {
  var historyCalls: [(path: String, bundleId: String, limit: Int)] = []
  var clipboardCalls: [(path: String, bundleId: String, limit: Int)] = []
  var historyResult: Int? = 7
  var clipboardResult: Int? = 11

  func importHandyHistory(path: String, bundleId: String, limit: Int) -> Int? {
    historyCalls.append((path, bundleId, limit))
    return historyResult
  }

  func importHandyClipboard(path: String, bundleId: String, limit: Int) -> Int? {
    clipboardCalls.append((path, bundleId, limit))
    return clipboardResult
  }
}

@main
struct InputiaHandyMemorySyncSelfCheck {
  static func main() {
    let base = URL(fileURLWithPath: "/tmp/inputia-handy-memory-sync")
    let paths = InputiaHandyDataPaths.defaultPaths(baseURL: base)
    let importer = MockImporter()
    let existing = Set([paths.history.path, paths.clipboard.path])

    let all = InputiaHandyMemorySync.sync(
      importer: importer,
      paths: paths,
      includeHistory: true,
      includeClipboard: true,
      limit: 42,
      fileExists: { existing.contains($0) }
    )

    let importsBothSources = all.history == .imported(7)
      && all.clipboard == .imported(11)
      && importer.historyCalls.first?.bundleId == "com.pais.handy"
      && importer.clipboardCalls.first?.bundleId == "com.pais.handy"
      && importer.historyCalls.first?.limit == 42
      && importer.clipboardCalls.first?.limit == 42

    let missingImporter = MockImporter()
    let missing = InputiaHandyMemorySync.sync(
      importer: missingImporter,
      paths: paths,
      includeHistory: true,
      includeClipboard: true,
      fileExists: { _ in false }
    )
    let missingSkipsImports = missing.history == .missing
      && missing.clipboard == .missing
      && missingImporter.historyCalls.isEmpty
      && missingImporter.clipboardCalls.isEmpty

    let clipboardOnlyImporter = MockImporter()
    let clipboardOnly = InputiaHandyMemorySync.sync(
      importer: clipboardOnlyImporter,
      paths: paths,
      includeHistory: false,
      includeClipboard: true,
      fileExists: { existing.contains($0) }
    )
    let supportsSourceSelection = clipboardOnly.history == .skipped
      && clipboardOnly.clipboard == .imported(11)
      && clipboardOnlyImporter.historyCalls.isEmpty
      && clipboardOnlyImporter.clipboardCalls.count == 1

    let failedImporter = MockImporter()
    failedImporter.historyResult = nil
    failedImporter.clipboardResult = nil
    let failed = InputiaHandyMemorySync.sync(
      importer: failedImporter,
      paths: paths,
      includeHistory: true,
      includeClipboard: true,
      fileExists: { existing.contains($0) }
    )
    let reportsFailures = failed.history == .failed && failed.clipboard == .failed

    let statusTextFindsData = InputiaHandyMemorySync.statusText(
      paths: paths,
      fileExists: { existing.contains($0) }
    ).hasPrefix("Handy 数据：")
    let statusTextReportsMissing = InputiaHandyMemorySync.statusText(
      paths: paths,
      fileExists: { _ in false }
    ).hasPrefix("未找到 Handy 数据：")

    let ok = importsBothSources
      && missingSkipsImports
      && supportsSourceSelection
      && reportsFailures
      && statusTextFindsData
      && statusTextReportsMissing

    print("handyMemorySyncSelfCheck=\(ok)")
    print("importsBothSources=\(importsBothSources)")
    print("missingSkipsImports=\(missingSkipsImports)")
    print("supportsSourceSelection=\(supportsSourceSelection)")
    print("reportsFailures=\(reportsFailures)")
    print("statusTextFindsData=\(statusTextFindsData)")
    print("statusTextReportsMissing=\(statusTextReportsMissing)")
    exit(ok ? 0 : 1)
  }
}
