import { platform, type } from "@tauri-apps/plugin-os";
import { type OSType } from "./utils/keyboard";

export type AppPlatform =
  | "macos"
  | "windows"
  | "linux"
  | "ios"
  | "android"
  | "unknown";

export const getSafePlatform = (): AppPlatform => {
  try {
    return platform() as AppPlatform;
  } catch (error) {
    console.warn("Failed to read Tauri platform:", error);
    return "unknown";
  }
};

export const getSafeOsType = (): OSType => {
  try {
    const osType = type();
    if (osType === "macos" || osType === "windows" || osType === "linux") {
      return osType;
    }
  } catch (error) {
    console.warn("Failed to read Tauri OS type:", error);
  }

  return "unknown";
};
