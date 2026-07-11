struct InputiaExpandedCandidateGridNavigation {
  static func rowCount(candidateCount: Int, columnCount: Int) -> Int {
    let columns = max(1, columnCount)
    guard candidateCount > 0 else {
      return 0
    }
    return (candidateCount + columns - 1) / columns
  }

  static func nextRow(currentRow: Int, candidateCount: Int, columnCount: Int) -> Int? {
    let rows = rowCount(candidateCount: candidateCount, columnCount: columnCount)
    let next = currentRow + 1
    return next < rows ? next : nil
  }

  static func previousRow(currentRow: Int) -> Int? {
    currentRow > 0 ? currentRow - 1 : nil
  }

  static func clampedRow(_ row: Int, candidateCount: Int, columnCount: Int) -> Int {
    let rows = rowCount(candidateCount: candidateCount, columnCount: columnCount)
    guard rows > 0 else {
      return 0
    }
    return min(max(0, row), rows - 1)
  }
}
