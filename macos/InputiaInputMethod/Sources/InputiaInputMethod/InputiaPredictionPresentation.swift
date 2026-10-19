import Foundation

/// 仅限制空输入联想的展示周期；普通候选、授权租约和插入准入由各自状态机管理。
struct InputiaPredictionPresentation {
  static let lifetime: TimeInterval = 5
  private var nextToken: UInt64 = 0
  private var startedAt: TimeInterval?
  private(set) var currentToken: UInt64?
  private(set) var deadline: TimeInterval?

  /// 只在空格选词实际完成上屏后开始；查询和策略续租不得调用此方法。
  @discardableResult
  mutating func begin(now: TimeInterval) -> UInt64? {
    invalidate()
    let expiry = now + Self.lifetime
    guard now.isFinite, now >= 0, expiry.isFinite, expiry > now,
      nextToken < UInt64.max else { return nil }
    nextToken += 1
    currentToken = nextToken
    startedAt = now
    deadline = expiry
    return nextToken
  }

  /// 查询入口和异步结果使用同一个 token；读取不会延长五秒截止时间。
  func isCurrent(_ token: UInt64, now: TimeInterval) -> Bool {
    guard currentToken == token, let startedAt, let deadline else { return false }
    return now.isFinite && now >= startedAt && now < deadline
  }

  /// Esc、新输入或焦点变化立即关闭，并让未返回的查询失效。
  mutating func invalidate() {
    currentToken = nil
    startedAt = nil
    deadline = nil
  }

  /// 定时器只可关闭自己所属且确实到期的会话，旧定时器不能影响新会话。
  @discardableResult
  mutating func expire(_ token: UInt64, now: TimeInterval) -> Bool {
    guard currentToken == token, let deadline, now.isFinite, now >= deadline else { return false }
    invalidate()
    return true
  }
}
