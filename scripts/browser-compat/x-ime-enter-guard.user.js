// ==UserScript==
// @name         Inputia：X/Grok 组合确认回车保护（待实机验证）
// @namespace    com.inputia.browser-compat
// @version      0.1.0
// @description  仅在 X 的 Grok 输入区阻止组合确认回车继续触发网页快捷键。
// @match        https://x.com/*
// @run-at       document-start
// @grant        none
// ==/UserScript==

(() => {
  "use strict";

  const installed = Symbol.for("com.inputia.x-ime-enter-guard.v1");
  if (window[installed]) return;
  Object.defineProperty(window, installed, { value: true });

  let composing = new WeakSet();
  let confirmingEnter = null;
  let currentPath = location.pathname;

  function reset() {
    composing = new WeakSet();
    confirmingEnter = null;
  }

  function isGrokPage() {
    if (currentPath !== location.pathname) {
      currentPath = location.pathname;
      reset();
    }
    const allowed =
      location.protocol === "https:" &&
      location.hostname === "x.com" &&
      (currentPath === "/i/grok" || currentPath.startsWith("/i/grok/"));
    if (!allowed) reset();
    return allowed;
  }

  // 只取得可编辑节点身份，不读取 value、textContent 或 composition.data。
  function editableTarget(event) {
    const target = event.composedPath().find((node) => node instanceof Element);
    if (!target) return null;
    if (target.tagName === "TEXTAREA") {
      return target.disabled || target.readOnly ? null : target;
    }
    if (target.tagName === "INPUT") {
      return !target.disabled &&
        !target.readOnly &&
        ["text", "search", "email", "url", "tel"].includes(target.type)
        ? target
        : null;
    }
    if (!target.isContentEditable) return null;
    return target.closest("[contenteditable]");
  }

  function enterKind(event) {
    if (event.code === "NumpadEnter") return "numpad";
    if (event.code === "Enter") return "main";
    if (event.key === "Enter" || event.keyCode === 13) return "unknown";
    return null;
  }

  function sameEnter(target, kind) {
    return (
      confirmingEnter?.target === target &&
      (confirmingEnter.kind === kind ||
        confirmingEnter.kind === "unknown" ||
        kind === "unknown")
    );
  }

  function handleKeyboard(event) {
    if (!isGrokPage()) return;
    const target = editableTarget(event);
    const kind = enterKind(event);
    if (
      !target ||
      !kind ||
      event.shiftKey ||
      event.ctrlKey ||
      event.altKey ||
      event.metaKey
    ) {
      confirmingEnter = null;
      return;
    }

    if (event.type === "keydown") {
      const confirmsComposition =
        event.isComposing || event.keyCode === 229 || composing.has(target);
      if (confirmsComposition || (event.repeat && sameEnter(target, kind))) {
        confirmingEnter = { target, kind };
      } else {
        // 漏收松键也不能吞掉下一次独立的普通 Enter。
        confirmingEnter = null;
      }
    }

    if (!sameEnter(target, kind)) {
      confirmingEnter = null;
      return;
    }

    // 保留浏览器/输入法默认上屏，只阻止网页处理这一轮确认键。
    event.stopImmediatePropagation();
    if (event.type === "keyup") confirmingEnter = null;
  }

  window.addEventListener(
    "compositionstart",
    (event) => {
      if (!isGrokPage()) return;
      confirmingEnter = null;
      const target = editableTarget(event);
      if (target) composing.add(target);
    },
    true,
  );
  window.addEventListener(
    "compositionend",
    (event) => {
      if (!isGrokPage()) return;
      const target = editableTarget(event);
      if (target) composing.delete(target);
    },
    true,
  );
  for (const type of ["keydown", "keypress", "keyup"]) {
    window.addEventListener(type, handleKeyboard, true);
  }
  for (const type of [
    "focusin",
    "focusout",
    "blur",
    "pagehide",
    "popstate",
    "hashchange",
  ]) {
    window.addEventListener(type, reset, true);
  }
})();
