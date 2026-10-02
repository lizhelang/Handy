// 复用输入法已有的连珠品牌源，不另行设计应用与托盘图标。
import { copyFileSync, readFileSync } from "node:fs";
import { spawnSync } from "node:child_process";

function run(command: string, args: string[], input?: string) {
  const result = spawnSync(command, args, { input, encoding: "utf8" });
  if (result.error || result.status !== 0) {
    throw result.error ?? new Error(result.stderr);
  }
}

const resourcesDir = "macos/InputiaInputMethod/Resources";
const logo = readFileSync(`${resourcesDir}/InputiaLogo.svg`, "utf8");
const logoPath = logo.match(/<path\b[^>]*\bd="([\s\S]*?)"/);
if (!logoPath) throw new Error("既有 Inputia 连珠标记缺少路径");

run("bun", [
  "run",
  "tauri",
  "icon",
  `${resourcesDir}/InputiaAppIcon.svg`,
  "--output",
  "src-tauri/icons",
]);
// macOS 使用已有原生图标，避免两个组件采用不同的渲染制品。
copyFileSync(`${resourcesDir}/Inputia.icns`, "src-tauri/icons/icon.icns");
copyFileSync("src-tauri/icons/icon.png", "src-tauri/icons/logo.png");
for (const [suffix, color] of [
  ["", "#ffffff"],
  ["_dark", "#222222"],
  ["_colored", "#2F6F73"],
]) {
  for (const state of ["idle", "recording", "transcribing", "idle_warning"]) {
    const center =
      state === "recording"
        ? `<circle cx="32" cy="32" r="10" fill="${color}"/>`
        : state === "transcribing"
          ? `<path d="M24 32h16M32 24v16" stroke="${color}" stroke-width="5" stroke-linecap="round"/>`
          : "";
    const warning =
      state === "idle_warning"
        ? `<circle cx="50" cy="50" r="12" fill="#e6ad39"/><path d="M50 42v8m0 5v1" stroke="#222" stroke-width="3" stroke-linecap="round"/>`
        : "";
    const svg = `<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64"><g transform="translate(4 4) scale(0.0546875)"><path fill="${color}" fill-rule="evenodd" d="${logoPath[1]}"/></g>${center}${warning}</svg>`;
    run(
      "rsvg-convert",
      ["-o", `src-tauri/resources/tray_${state}${suffix}.png`],
      svg,
    );
  }
}
