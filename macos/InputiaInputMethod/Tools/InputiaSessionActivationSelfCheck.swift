import Darwin
import Foundation

@main
struct InputiaSessionActivationSelfCheck {
  final class Session {
    var activation = InputiaSessionActivation()
  }

  /// 与输入控制器相同的规则：激活即成为活动会话，停用只清自己的标记，
  /// 收到按键时由 InputiaSessionActivation 决定是否收回活动标记。
  final class Host {
    var active: Session?
    func activate(_ session: Session) {
      session.activation.activate()
      active = session
    }
    func deactivate(_ session: Session) {
      session.activation.deactivate()
      if active === session { active = nil }
    }
    /// 返回候选窗此刻是否归收到按键的会话。
    func key(_ session: Session) -> Bool {
      if session.activation.shouldReclaim(activeIsSelf: active === session) { active = session }
      return active === session
    }
  }

  static func main() {
    var checks: [(String, Bool)] = []

    // Safari 网页输入框：激活 A、激活 B、停用 B，按键继续发给 A。
    do {
      let host = Host(), a = Session(), b = Session()
      host.activate(a); host.activate(b); host.deactivate(b)
      checks.append(("interleavedDeactivationLeavesNoOwner", host.active == nil))
      checks.append(("keyedActivatedSessionReclaimsPanel", host.key(a)))
      checks.append(("deactivatedSessionCannotReclaim", !host.key(b) && host.active === a))
    }

    // 两个会话都在激活期：按键送到哪个会话，候选窗就归哪个。
    do {
      let host = Host(), a = Session(), b = Session()
      host.activate(a); host.activate(b)
      checks.append(("olderActivatedSessionTakesPanelOnKey", host.key(a) && host.active === a))
      checks.append(("ownershipFollowsLaterKeys", host.key(b) && host.active === b))
    }

    // 正常切换（先停用再激活）不需要收回；旧会话不能抢回候选窗。
    do {
      let host = Host(), a = Session(), b = Session()
      host.activate(a); host.deactivate(a); host.activate(b)
      checks.append(("normalSwitchKeepsNewSession", host.key(b)))
      checks.append(("staleSessionAfterNormalSwitchCannotPaint", !host.key(a) && host.active === b))
    }

    // 从未激活过的会话收到按键也不收回。
    do {
      let host = Host(), a = Session(), fresh = Session()
      host.activate(a)
      checks.append(("neverActivatedSessionCannotReclaim", !host.key(fresh) && host.active === a))
    }

    // 活动会话自己的按键不触发收回；再次激活后照常工作。
    do {
      let host = Host(), a = Session()
      host.activate(a)
      checks.append(("activeSessionDoesNotReclaim", !a.activation.shouldReclaim(activeIsSelf: true)))
      host.deactivate(a); host.activate(a)
      checks.append(("reactivatedSessionOwnsPanel", host.key(a)))
    }

    let ok = checks.allSatisfy { $0.1 }
    print("sessionActivationSelfCheck=\(ok)")
    for (name, passed) in checks { print("\(name)=\(passed)") }
    exit(ok ? 0 : 1)
  }
}
