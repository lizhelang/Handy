import { invoke } from "@tauri-apps/api/core";
import { checkMicrophonePermission } from "tauri-plugin-macos-permissions-api";

interface MicrophonePermissionOptions {
  allowHardwareProbe?: boolean;
}

export const hasMacMicrophoneAccess = async ({
  allowHardwareProbe = false,
}: MicrophonePermissionOptions = {}): Promise<boolean> => {
  const pluginGranted = await checkMicrophonePermission();

  if (pluginGranted) {
    return true;
  }

  if (!allowHardwareProbe) {
    return false;
  }

  try {
    return await invoke<boolean>("has_microphone_input_access");
  } catch (error) {
    console.warn("Failed to probe microphone input access:", error);
    return false;
  }
};
