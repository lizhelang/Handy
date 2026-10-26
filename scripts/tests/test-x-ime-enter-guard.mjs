// 执行真实 userscript；事件替身只证明脚本合同，不代表 Safari/X 实机验收。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(
  new URL("../browser-compat/x-ime-enter-guard.user.js", import.meta.url),
  "utf8",
);

class Element {
  constructor(tagName = "TEXTAREA", options = {}) {
    Object.assign(
      this,
      {
        tagName,
        type: "text",
        disabled: false,
        readOnly: false,
        isContentEditable: false,
        editableRoot: null,
      },
      options,
    );
    for (const name of ["value", "textContent", "innerText", "innerHTML"]) {
      Object.defineProperty(this, name, {
        get() {
          throw new Error(`不得读取正文 ${name}`);
        },
      });
    }
  }

  closest(selector) {
    assert.equal(selector, "[contenteditable]");
    return this.editableRoot ?? (this.isContentEditable ? this : null);
  }
}

function fixture(pathname = "/i/grok") {
  const listeners = new Map();
  const window = {
    addEventListener(type, listener, capture) {
      assert.equal(capture, true);
      const entries = listeners.get(type) ?? [];
      entries.push(listener);
      listeners.set(type, entries);
    },
  };
  const location = { protocol: "https:", hostname: "x.com", pathname };
  const context = vm.createContext({ window, location, Element });
  const inject = () => vm.runInContext(source, context);
  inject();
  const textarea = new Element();
  let forwarded = 0;
  function emit(type, options = {}) {
    const event = {
      type,
      target: textarea,
      key: "Enter",
      code: "Enter",
      keyCode: 13,
      isComposing: false,
      repeat: false,
      shiftKey: false,
      ctrlKey: false,
      altKey: false,
      metaKey: false,
      stopped: false,
      defaultPrevented: false,
      ...options,
      composedPath() {
        return [this.target, window];
      },
      stopImmediatePropagation() {
        this.stopped = true;
      },
      preventDefault() {
        throw new Error("不得取消输入法默认上屏");
      },
    };
    Object.defineProperty(event, "data", {
      get() {
        throw new Error("不得读取组合正文");
      },
    });
    for (const listener of listeners.get(type) ?? []) {
      listener(event);
      if (event.stopped) break;
    }
    if (!event.stopped && ["keydown", "keypress", "keyup"].includes(type)) {
      forwarded += 1;
    }
    assert.equal(event.defaultPrevented, false);
    return event;
  }
  function assertPass(type, options) {
    assert.equal(emit(type, options).stopped, false);
  }
  function assertStop(type, options) {
    assert.equal(emit(type, options).stopped, true);
  }
  return {
    emit,
    assertPass,
    assertStop,
    textarea,
    location,
    listeners,
    inject,
    forwarded: () => forwarded,
  };
}

test("旧 WebKit 顺序：compositionend 后的 229 回车完整一轮受保护", () => {
  const f = fixture();
  f.emit("compositionstart");
  f.emit("compositionend");
  f.assertStop("keydown", { keyCode: 229 });
  f.assertStop("keypress");
  f.assertStop("keyup");
  assert.equal(f.forwarded(), 0);
  f.assertPass("keydown");
  f.assertPass("keypress");
  f.assertPass("keyup");
  assert.equal(f.forwarded(), 3);
});

test("标准顺序：正在组合的 Enter 与后续松键受保护", () => {
  const f = fixture();
  f.emit("compositionstart");
  f.assertStop("keydown", { isComposing: true });
  f.emit("compositionend");
  f.assertStop("keypress");
  f.assertStop("keyup");
  f.assertPass("keydown");
});

test("只凭 isComposing 或同目标 composition 状态也能保护", () => {
  const f = fixture();
  f.assertStop("keydown", { isComposing: true });
  f.assertStop("keyup");
  f.emit("compositionstart");
  f.assertStop("keydown");
  f.emit("compositionend");
  f.assertStop("keyup");
});

test("未观察到 compositionstart 时，229 仍能标记确认回车", () => {
  const f = fixture();
  f.assertStop("keydown", { keyCode: 229 });
  f.assertStop("keyup");
  f.assertPass("keydown");
});

test("无组合普通 Enter 和孤立 keyup 全部放行", () => {
  const f = fixture();
  f.assertPass("keydown");
  f.assertPass("keypress");
  f.assertPass("keyup");
});

test("数字键盘 Enter 和缺少 code 的 Enter 保留相同行为", () => {
  for (const code of ["NumpadEnter", ""]) {
    const f = fixture();
    f.assertStop("keydown", { code, keyCode: 229 });
    f.assertStop("keyup", { code });
    f.assertPass("keydown", { code });
  }
});

test("按住确认回车连发持续保护，松开后的普通 Enter 放行", () => {
  const f = fixture();
  f.assertStop("keydown", { keyCode: 229 });
  f.assertStop("keydown", { repeat: true });
  f.assertStop("keypress", { repeat: true });
  f.assertStop("keyup");
  f.assertPass("keydown");
});

test("漏收 keyup 不吞下一次独立 Enter", () => {
  const f = fixture();
  f.assertStop("keydown", { keyCode: 229 });
  f.assertPass("keydown");
  f.assertPass("keypress");
  f.assertPass("keyup");
});

test("其他按键清除旧回车轮次", () => {
  const f = fixture();
  f.assertStop("keydown", { keyCode: 229 });
  f.assertPass("keydown", { key: "Escape", code: "Escape", keyCode: 27 });
  f.assertPass("keyup");
});

test("Shift/Ctrl/Alt/Meta 修饰 Enter 不被拦截且清旧轮", () => {
  for (const modifier of ["shiftKey", "ctrlKey", "altKey", "metaKey"]) {
    const f = fixture();
    f.assertStop("keydown", { keyCode: 229 });
    f.assertPass("keydown", { keyCode: 229, [modifier]: true });
    f.assertPass("keyup");
  }
});

test("非可编辑、只读、禁用和密码输入节点不受保护", () => {
  for (const target of [
    new Element("BUTTON"),
    new Element("TEXTAREA", { readOnly: true }),
    new Element("TEXTAREA", { disabled: true }),
    new Element("INPUT", { type: "password" }),
    new Element("INPUT", { type: "checkbox" }),
  ]) {
    const f = fixture();
    f.assertPass("keydown", { target, keyCode: 229 });
    f.assertPass("keyup", { target });
  }
});

test("文本输入和 contenteditable 后代按编辑根节点配对", () => {
  for (const target of [
    new Element("INPUT"),
    new Element("DIV", { isContentEditable: true }),
  ]) {
    const f = fixture();
    const child =
      target.tagName === "DIV"
        ? new Element("SPAN", { isContentEditable: true, editableRoot: target })
        : target;
    f.assertStop("keydown", { target: child, keyCode: 229 });
    f.assertStop("keyup", { target });
  }
});

test("不同目标不继承旧回车保护", () => {
  const f = fixture();
  f.assertStop("keydown", { keyCode: 229 });
  f.assertPass("keyup", { target: new Element() });
  f.assertPass("keyup");
});

test("不同物理 Enter 不继承旧回车保护", () => {
  const f = fixture();
  f.assertStop("keydown", { keyCode: 229 });
  f.assertPass("keyup", { code: "NumpadEnter" });
  f.assertPass("keyup");
});

test("焦点、窗口和导航边界撤销组合状态及旧回车轮次", () => {
  for (const type of [
    "focusin",
    "focusout",
    "blur",
    "pagehide",
    "popstate",
    "hashchange",
  ]) {
    const f = fixture();
    f.emit("compositionstart");
    f.assertStop("keydown");
    f.emit(type);
    f.assertPass("keyup");
    f.assertPass("keydown");
  }
});

test("非 Grok 路径不处理，SPA 进入 Grok 后开始处理", () => {
  const f = fixture("/home");
  f.emit("compositionstart");
  f.assertPass("keydown", { keyCode: 229 });
  f.location.pathname = "/i/grok";
  f.assertPass("keydown");
  f.assertStop("keydown", { keyCode: 229 });
  f.assertStop("keyup");
});

test("Grok 子路径支持，类似前缀与其他来源拒绝", () => {
  const f = fixture("/i/grok/example");
  f.assertStop("keydown", { keyCode: 229 });
  f.location.pathname = "/i/grok-other";
  f.assertPass("keyup");
  f.assertPass("keydown", { keyCode: 229 });
  f.location.pathname = "/i/grok";
  f.location.hostname = "example.com";
  f.assertPass("keydown", { keyCode: 229 });
  f.location.hostname = "x.com";
  f.location.protocol = "http:";
  f.assertPass("keydown", { keyCode: 229 });
});

test("离开和返回 Grok 清除旧组合及未完成的确认键", () => {
  const f = fixture();
  f.emit("compositionstart");
  f.assertStop("keydown");
  f.location.pathname = "/home";
  f.assertPass("keyup");
  f.location.pathname = "/i/grok";
  f.assertPass("keydown");
});

test("重新开始组合不能继承上一轮回车保护", () => {
  const f = fixture();
  f.assertStop("keydown", { keyCode: 229 });
  f.emit("compositionstart");
  f.assertPass("keyup");
  f.emit("compositionend");
  f.assertPass("keydown");
});

test("重复注入不重复注册事件处理器", () => {
  const f = fixture();
  const count = [...f.listeners.values()].reduce(
    (total, values) => total + values.length,
    0,
  );
  f.inject();
  assert.equal(
    [...f.listeners.values()].reduce(
      (total, values) => total + values.length,
      0,
    ),
    count,
  );
  f.assertStop("keydown", { keyCode: 229 });
  f.assertStop("keyup");
});
