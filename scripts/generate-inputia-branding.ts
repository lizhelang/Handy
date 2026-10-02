// 从唯一品牌源生成应用与托盘资源；需要本机 rsvg-convert。
import { copyFileSync, existsSync, unlinkSync } from "node:fs";
import { spawnSync } from "node:child_process";

function run(command: string, args: string[], input?: string) {
  const result = spawnSync(command, args, { input, encoding: "utf8" });
  if (result.error || result.status !== 0) {
    throw result.error ?? new Error(result.stderr);
  }
}

run("bun", [
  "run",
  "tauri",
  "icon",
  "src-tauri/icons/inputia.svg",
  "--output",
  "src-tauri/icons",
]);
copyFileSync("src-tauri/icons/icon.png", "src-tauri/icons/logo.png");
for (const [suffix, color] of [
  ["", "#ffffff"],
  ["_dark", "#222222"],
  ["_colored", "#2f7f83"],
]) {
  for (const state of ["idle", "recording", "transcribing", "idle_warning"]) {
    const center =
      state === "recording"
        ? `<circle cx="32" cy="32" r="10" fill="${color}"/>`
        : state === "transcribing"
          ? `<path d="M24 32h16M32 24v16" stroke="${color}" stroke-width="5" stroke-linecap="round"/>`
          : `<circle cx="32" cy="32" r="6" fill="none" stroke="${color}" stroke-width="4"/>`;
    const warning =
      state === "idle_warning"
        ? `<circle cx="50" cy="50" r="12" fill="#e6ad39"/><path d="M50 42v8m0 5v1" stroke="#222" stroke-width="3" stroke-linecap="round"/>`
        : "";
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><circle cx="32" cy="32" r="23" fill="none" stroke="${color}" stroke-width="6"/>${center}${warning}</svg>`;
    run(
      "rsvg-convert",
      ["-o", `src-tauri/resources/tray_${state}${suffix}.png`],
      svg,
    );
  }
}
// 旧品牌资源不再进入安装包。
for (const filename of [
  "handy.png",
  "handy_warning.png",
  "recording.png",
  "transcribing.png",
]) {
  const path = `src-tauri/resources/${filename}`;
  if (existsSync(path)) unlinkSync(path);
}
