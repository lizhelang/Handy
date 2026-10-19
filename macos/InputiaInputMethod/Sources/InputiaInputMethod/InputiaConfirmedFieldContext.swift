import Foundation

/// 仅保存本次输入会话自己已确认插入的文本；新目标只能证明最新片段。
struct InputiaConfirmedFieldContext {
  struct Field: Equatable {
    let server: String
    let instance: String
  }
  struct Scope: Equatable {
    let client: ObjectIdentifier
    let activation: UInt64
  }
  private struct Confirmed {
    let field: Field
    let scope: Scope
    let text: String
    let end: Int
  }
  private struct Pending {
    let token: UInt64
    let scope: Scope
    let text: String
    let start: Int
    let end: Int
    var prefix: Confirmed?
  }
  private var confirmed: Confirmed?
  private var pending: Pending?
  private(set) var generation: UInt64 = 0
  var pendingToken: UInt64? { pending?.token }
  var hasContext: Bool { confirmed != nil || pending != nil }

  static func bounded(_ text: String, limit: Int = 512) -> String {
    String(String.UnicodeScalarView(text.unicodeScalars.suffix(limit)))
  }

  mutating func reset() {
    generation &+= 1
    confirmed = nil; pending = nil
  }

  /// marked 文本不是已上屏文本；只允许其从已确认正文尾部开始。
  mutating func observe(scope: Scope, selection: NSRange, marked: NSRange) {
    guard let previousScope = pending?.scope ?? confirmed?.scope,
      let end = pending?.end ?? confirmed?.end else { return }
    guard scope == previousScope, selection.location != NSNotFound, selection.length == 0 else {
      reset(); return
    }
    if marked.location != NSNotFound && marked.length > 0 {
      let (markedEnd, overflow) = marked.location.addingReportingOverflow(marked.length)
      if overflow || marked.location != end || selection.location < end || selection.location > markedEnd { reset() }
    } else if selection.location != end { reset() }
  }

  @discardableResult
  mutating func committed(_ text: String, start: Int, selection: NSRange, scope: Scope) -> UInt64? {
    let (end, overflow) = start.addingReportingOverflow(text.utf16.count)
    guard !text.isEmpty, !text.unicodeScalars.contains(where: { CharacterSet.controlCharacters.contains($0) }),
      start >= 0, start != NSNotFound, !overflow, selection == NSRange(location: end, length: 0) else {
      reset(); return nil
    }
    let prefix = confirmed.flatMap { $0.scope == scope && $0.end == start ? $0 : nil }
    generation &+= 1
    pending = Pending(token: generation, scope: scope, text: Self.bounded(text), start: start, end: end, prefix: prefix)
    // 尚未证实的上一段不能被下一次捕获追认。
    confirmed = nil
    return generation
  }

  mutating func resolve(token: UInt64, field: Field?, scope: Scope, selection: NSRange) -> String? {
    guard let pending, pending.token == token, pending.scope == scope,
      selection == NSRange(location: pending.end, length: 0) else { return nil }
    guard let field, !field.server.isEmpty, !field.instance.isEmpty else {
      confirmed = nil
      self.pending?.prefix = nil
      return pending.text
    }
    let prefix = pending.prefix.flatMap {
      $0.field == field && $0.scope == scope && $0.end == pending.start ? $0.text : nil
    } ?? ""
    if prefix.isEmpty { self.pending?.prefix = nil }
    let text = Self.bounded(prefix + pending.text)
    confirmed = Confirmed(field: field, scope: scope, text: text, end: pending.end)
    return text
  }
}
