import Foundation

/// 单个输入会话的激活期。
///
/// 宿主（如 Safari 网页输入框）会交错激活多个会话：先激活 A、再激活 B、随后停用 B，
/// 却继续把按键发给 A。候选窗只归活动会话所有，所以仍在激活期的会话收到按键时要收回活动标记；
/// 已停用或从未激活的会话不收回，不能抢占其他会话的候选窗。
struct InputiaSessionActivation {
  private(set) var isActivated = false

  mutating func activate() { isActivated = true }
  mutating func deactivate() { isActivated = false }

  /// 收到宿主按键时是否收回活动标记。
  func shouldReclaim(activeIsSelf: Bool) -> Bool { isActivated && !activeIsSelf }
}
