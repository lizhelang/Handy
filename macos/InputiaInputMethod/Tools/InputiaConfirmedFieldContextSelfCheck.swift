import Foundation

@main
enum InputiaConfirmedFieldContextSelfCheck {
  static func main() {
    final class Client {}
    let client = Client(), otherClient = Client()
    let scope = InputiaConfirmedFieldContext.Scope(client: ObjectIdentifier(client), activation: 1)
    let field = InputiaConfirmedFieldContext.Field(server: "service-a", instance: "field-a")
    let otherField = InputiaConfirmedFieldContext.Field(server: "service-a", instance: "field-b")
    var checks = 0
    func check(_ value: Bool) { precondition(value); checks += 1 }
    func commit(_ context: inout InputiaConfirmedFieldContext, _ text: String, _ start: Int) -> UInt64 {
      context.committed(text, start: start,
        selection: NSRange(location: start + text.utf16.count, length: 0), scope: scope)!
    }
    func resolve(_ context: inout InputiaConfirmedFieldContext, _ token: UInt64,
      _ end: Int, _ proof: InputiaConfirmedFieldContext.Field? = nil) -> String? {
      context.resolve(token: token, field: proof, scope: scope, selection: NSRange(location: end, length: 0))
    }
    func seeded() -> InputiaConfirmedFieldContext {
      var context = InputiaConfirmedFieldContext()
      let token = commit(&context, "你好", 0)
      precondition(resolve(&context, token, 2, field) == "你好")
      return context
    }
    var context = seeded()
    var token = commit(&context, "世界", 2)
    check(resolve(&context, token, 4, field) == "你好世界")
    token = commit(&context, "朋友", 4)
    check(resolve(&context, token, 6, field) == "你好世界朋友")
    token = commit(&context, "们", 6)
    check(resolve(&context, token, 7, field) == "你好世界朋友们")

    // 同一 IMK proxy 和连续位置不足以证明字段；必须匹配服务返回的实例。
    context = seeded(); token = commit(&context, "别处", 2)
    check(resolve(&context, token, 4, otherField) == "别处")
    context = seeded(); token = commit(&context, "重启", 2)
    check(resolve(&context, token, 4, .init(server: "service-b", instance: field.instance)) == "重启")
    context = seeded(); token = commit(&context, "间隔", 9)
    check(resolve(&context, token, 11, field) == "间隔")

    context = InputiaConfirmedFieldContext()
    _ = commit(&context, "未证明", 0)
    token = commit(&context, "新段", 3)
    check(resolve(&context, token, 5, field) == "新段")
    context = seeded(); token = commit(&context, "无证明", 2)
    check(resolve(&context, token, 5) == "无证明")
    token = commit(&context, "之后", 5)
    check(resolve(&context, token, 7, field) == "之后")

    context = seeded()
    context.observe(scope: scope, selection: NSRange(location: 4, length: 0), marked: NSRange(location: 2, length: 2))
    token = commit(&context, "世界", 2)
    check(resolve(&context, token, 4, field) == "你好世界")
    context = seeded()
    context.observe(scope: scope, selection: NSRange(location: 1, length: 0), marked: NSRange(location: NSNotFound, length: 0))
    token = commit(&context, "替换", 1)
    check(resolve(&context, token, 3, field) == "替换")
    context = seeded()
    context.observe(scope: .init(client: ObjectIdentifier(otherClient), activation: 1),
      selection: NSRange(location: 2, length: 0), marked: NSRange(location: NSNotFound, length: 0))
    token = commit(&context, "新框", 2)
    check(resolve(&context, token, 4, field) == "新框")
    context = seeded()
    context.observe(scope: .init(client: scope.client, activation: 2),
      selection: NSRange(location: 2, length: 0), marked: NSRange(location: NSNotFound, length: 0))
    token = commit(&context, "新会话", 2)
    check(resolve(&context, token, 5, field) == "新会话")

    context = seeded(); token = commit(&context, "取消", 2); context.reset()
    check(resolve(&context, token, 4, field) == nil)
    context = seeded()
    check(context.committed("错误", start: 2, selection: NSRange(location: 3, length: 0), scope: scope) == nil)
    token = commit(&context, "重来", 3)
    check(resolve(&context, token, 5, field) == "重来")

    // 与 Rust chars 一致按 Unicode scalar 截断；不能按 grapheme 放过超长组合字符。
    let long = String(repeating: "汉", count: 500) + String(repeating: "e\u{301}", count: 20)
    context = InputiaConfirmedFieldContext(); token = commit(&context, long, 0)
    let bounded = resolve(&context, token, long.utf16.count, field)!
    check(bounded.unicodeScalars.count == 512 && bounded.utf8.count <= 2048)
    check(InputiaConfirmedFieldContext.bounded(bounded, limit: 64).unicodeScalars.count == 64)
    print("confirmedFieldContextSelfCheck=true checks=\(checks)")
  }
}
