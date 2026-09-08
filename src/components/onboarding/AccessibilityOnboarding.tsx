import { useEffect, useState, useCallback, useRef } from "react";
import { useTranslation } from "react-i18next";
import { getName } from "@tauri-apps/api/app";
import { platform } from "@tauri-apps/plugin-os";
import {
  checkAccessibilityPermission,
  requestAccessibilityPermission,
  checkMicrophonePermission,
  requestMicrophonePermission,
} from "tauri-plugin-macos-permissions-api";
import { toast } from "sonner";
import { commands } from "@/bindings";
import { useSettingsStore } from "@/stores/settingsStore";
import InputiaWordmark from "../icons/InputiaWordmark";
import {
  Keyboard,
  Mic,
  Check,
  Loader2,
  RefreshCw,
  AlertCircle,
} from "lucide-react";

interface AccessibilityOnboardingProps {
  onComplete: () => void;
}

type PermissionStatus = "checking" | "needed" | "waiting" | "granted" | "error";
type PermissionPlatform = "macos" | "windows" | "other";

interface PermissionsState {
  accessibility: PermissionStatus;
  microphone: PermissionStatus;
}

const AccessibilityOnboarding: React.FC<AccessibilityOnboardingProps> = ({
  onComplete,
}) => {
  const { t } = useTranslation();
  const refreshAudioDevices = useSettingsStore(
    (state) => state.refreshAudioDevices,
  );
  const refreshOutputDevices = useSettingsStore(
    (state) => state.refreshOutputDevices,
  );
  const [permissionPlatform, setPermissionPlatform] =
    useState<PermissionPlatform | null>(null);
  const [permissions, setPermissions] = useState<PermissionsState>({
    accessibility: "checking",
    microphone: "checking",
  });
  const [currentAppName, setCurrentAppName] = useState<string | null>(null);
  const [isRechecking, setIsRechecking] = useState(false);
  const [initializationFailed, setInitializationFailed] = useState(false);
  const [inputReady, setInputReady] = useState(false);
  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const errorCountRef = useRef<number>(0);
  const MAX_POLLING_ERRORS = 3;

  const isMacOS = permissionPlatform === "macos";
  const isWindows = permissionPlatform === "windows";
  const showMicrophonePermission = isMacOS || isWindows;
  const showAccessibilityPermission = isMacOS;

  const allGranted = isMacOS
    ? permissions.accessibility === "granted" &&
      permissions.microphone === "granted" &&
      inputReady &&
      !initializationFailed
    : isWindows
      ? permissions.microphone === "granted"
      : true;

  const completeOnboarding = useCallback(async () => {
    await Promise.all([refreshAudioDevices(), refreshOutputDevices()]);
    timeoutRef.current = setTimeout(() => onComplete(), 300);
  }, [onComplete, refreshAudioDevices, refreshOutputDevices]);

  const initializeMacOSInput = useCallback(async () => {
    try {
      const [enigoResult, shortcutsResult] = await Promise.all([
        commands.initializeEnigo(),
        commands.initializeShortcuts(),
      ]);
      return enigoResult.status === "ok" && shortcutsResult.status === "ok";
    } catch (e) {
      console.warn("Failed to initialize after permission grant:", e);
      return false;
    }
  }, []);

  const checkMacOSPermissions = useCallback(
    async ({ notifyOnCheckError = true } = {}) => {
      setInputReady(false);
      const [accessibilityResult, microphoneResult] = await Promise.allSettled([
        checkAccessibilityPermission(),
        checkMicrophonePermission(),
      ]);

      const accessibilityGranted =
        accessibilityResult.status === "fulfilled" && accessibilityResult.value;
      const microphoneGranted =
        microphoneResult.status === "fulfilled" && microphoneResult.value;
      const hasCheckError =
        accessibilityResult.status === "rejected" ||
        microphoneResult.status === "rejected";

      const newState: PermissionsState = {
        accessibility:
          accessibilityResult.status === "rejected"
            ? "error"
            : accessibilityGranted
              ? "granted"
              : "needed",
        microphone:
          microphoneResult.status === "rejected"
            ? "error"
            : microphoneGranted
              ? "granted"
              : "needed",
      };

      setPermissions(newState);

      if (hasCheckError) {
        setInitializationFailed(false);
        console.error("Failed to check macOS permissions:", {
          accessibility:
            accessibilityResult.status === "rejected"
              ? accessibilityResult.reason
              : null,
          microphone:
            microphoneResult.status === "rejected"
              ? microphoneResult.reason
              : null,
        });
        if (notifyOnCheckError) {
          toast.error(t("onboarding.permissions.errors.checkFailed"));
        }
        return {
          accessibilityGranted,
          microphoneGranted,
          hasCheckError,
        };
      }

      if (!(accessibilityGranted && microphoneGranted)) {
        setInitializationFailed(false);
        return {
          accessibilityGranted,
          microphoneGranted,
          hasCheckError,
        };
      }

      const initialized = await initializeMacOSInput();
      setInitializationFailed(!initialized);
      if (!initialized) {
        return {
          accessibilityGranted,
          microphoneGranted,
          hasCheckError,
        };
      }

      try {
        await completeOnboarding();
        setInputReady(true);
      } catch (error) {
        setInitializationFailed(true);
        console.error("Failed to complete permissions onboarding:", error);
      }

      return {
        accessibilityGranted,
        microphoneGranted,
        hasCheckError,
      };
    },
    [completeOnboarding, initializeMacOSInput, t],
  );

  const hasWindowsMicrophoneAccess = useCallback(async (): Promise<boolean> => {
    const microphoneStatus =
      await commands.getWindowsMicrophonePermissionStatus();

    if (!microphoneStatus.supported) {
      return true;
    }

    return microphoneStatus.overall_access !== "denied";
  }, []);

  // Check platform and permission status on mount
  useEffect(() => {
    const currentPlatform = platform();
    const nextPlatform: PermissionPlatform =
      currentPlatform === "macos"
        ? "macos"
        : currentPlatform === "windows"
          ? "windows"
          : "other";

    setPermissionPlatform(nextPlatform);

    // Skip immediately on unsupported platforms
    if (nextPlatform === "other") {
      onComplete();
      return;
    }

    if (nextPlatform === "macos") {
      getName()
        .then((name) => setCurrentAppName(name))
        .catch((error) => {
          console.warn("Failed to read current app name:", error);
        });
    }

    const checkInitial = async () => {
      if (nextPlatform === "macos") {
        await checkMacOSPermissions();
        return;
      }

      try {
        const microphoneGranted = await hasWindowsMicrophoneAccess();

        setPermissions({
          accessibility: "granted",
          microphone: microphoneGranted ? "granted" : "needed",
        });

        if (microphoneGranted) {
          await completeOnboarding();
        }
      } catch (error) {
        console.warn("Failed to check Windows microphone permissions:", error);
        setPermissions({
          accessibility: "granted",
          microphone: "granted",
        });
        await completeOnboarding();
      }
    };

    checkInitial();
  }, [
    checkMacOSPermissions,
    completeOnboarding,
    hasWindowsMicrophoneAccess,
    onComplete,
  ]);

  // Polling for permissions after user clicks a button
  const startPolling = useCallback(() => {
    if (pollingRef.current || permissionPlatform === null) return;

    pollingRef.current = setInterval(async () => {
      try {
        if (permissionPlatform === "windows") {
          const microphoneGranted = await hasWindowsMicrophoneAccess();

          if (microphoneGranted) {
            setPermissions((prev) => ({ ...prev, microphone: "granted" }));

            if (pollingRef.current) {
              clearInterval(pollingRef.current);
              pollingRef.current = null;
            }

            await completeOnboarding();
          }

          errorCountRef.current = 0;
          return;
        }

        const result = await checkMacOSPermissions({
          notifyOnCheckError: false,
        });

        // If both granted, stop polling, refresh audio devices, and proceed
        if (result.accessibilityGranted && result.microphoneGranted) {
          if (pollingRef.current) {
            clearInterval(pollingRef.current);
            pollingRef.current = null;
          }
        }

        if (result.hasCheckError) {
          errorCountRef.current += 1;

          if (errorCountRef.current >= MAX_POLLING_ERRORS) {
            if (pollingRef.current) {
              clearInterval(pollingRef.current);
              pollingRef.current = null;
            }
            toast.error(t("onboarding.permissions.errors.checkFailed"));
          }
        } else {
          errorCountRef.current = 0;
        }
      } catch (error) {
        console.error("Error checking permissions:", error);
        errorCountRef.current += 1;

        if (errorCountRef.current >= MAX_POLLING_ERRORS) {
          // Stop polling after too many consecutive errors
          if (pollingRef.current) {
            clearInterval(pollingRef.current);
            pollingRef.current = null;
          }
          toast.error(t("onboarding.permissions.errors.checkFailed"));
        }
      }
    }, 1000);
  }, [
    completeOnboarding,
    checkMacOSPermissions,
    hasWindowsMicrophoneAccess,
    permissionPlatform,
    t,
  ]);

  // Cleanup polling and timeouts on unmount
  useEffect(() => {
    return () => {
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
      }
      if (timeoutRef.current) {
        clearTimeout(timeoutRef.current);
      }
    };
  }, []);

  const handleGrantAccessibility = async () => {
    try {
      await requestAccessibilityPermission();
      setPermissions((prev) => ({ ...prev, accessibility: "waiting" }));
      startPolling();
    } catch (error) {
      console.error("Failed to request accessibility permission:", error);
      toast.error(t("onboarding.permissions.errors.requestFailed"));
    }
  };

  const handleGrantMicrophone = async () => {
    try {
      if (isWindows) {
        await commands.openMicrophonePrivacySettings();
      } else {
        await requestMicrophonePermission();
      }

      setPermissions((prev) => ({ ...prev, microphone: "waiting" }));
      startPolling();
    } catch (error) {
      console.error("Failed to request microphone permission:", error);
      toast.error(t("onboarding.permissions.errors.requestFailed"));
    }
  };

  const handleRecheckPermissions = async () => {
    if (!isMacOS) return;

    try {
      setIsRechecking(true);
      await checkMacOSPermissions();
    } finally {
      setIsRechecking(false);
    }
  };

  const isChecking =
    permissionPlatform === null ||
    (isMacOS &&
      permissions.accessibility === "checking" &&
      permissions.microphone === "checking") ||
    (isWindows && permissions.microphone === "checking");

  // Still checking platform/initial permissions
  if (isChecking) {
    return (
      <div className="h-screen w-screen flex items-center justify-center">
        <Loader2 className="w-8 h-8 animate-spin text-text/50" />
      </div>
    );
  }

  // All permissions granted - show success briefly
  if (allGranted) {
    return (
      <div className="h-screen w-screen flex flex-col items-center justify-center gap-4">
        <div className="p-4 rounded-full bg-emerald-500/20">
          <Check className="w-12 h-12 text-emerald-400" />
        </div>
        <p className="text-lg font-medium text-text">
          {t("onboarding.permissions.allGranted")}
        </p>
      </div>
    );
  }

  // Show permissions request screen
  return (
    <div className="h-screen w-screen flex flex-col p-6 gap-6 items-center justify-center">
      <div className="flex flex-col items-center gap-2">
        <InputiaWordmark size="hero" />
      </div>

      <div className="max-w-md w-full flex flex-col items-center gap-4">
        <div className="text-center mb-2">
          <h2 className="text-xl font-semibold text-text mb-2">
            {t("onboarding.permissions.title")}
          </h2>
          <p className="text-text/70">
            {t("onboarding.permissions.description")}
          </p>
          {currentAppName && (
            <p className="text-text/60 text-sm mt-2">
              {t("onboarding.permissions.currentApp", {
                appName: currentAppName,
              })}
            </p>
          )}
          {initializationFailed && (
            <p className="text-amber-400 text-sm mt-3">
              {t("onboarding.permissions.errors.initializeFailed")}
            </p>
          )}
        </div>

        {/* Microphone Permission Card */}
        {showMicrophonePermission && (
          <div className="w-full p-4 rounded-lg bg-white/5 border border-mid-gray/20">
            <div className="flex items-center gap-4">
              <div className="p-3 rounded-full bg-logo-primary/20 shrink-0">
                <Mic className="w-6 h-6 text-accent-text" />
              </div>
              <div className="flex-1 min-w-0">
                <h3 className="font-medium text-text">
                  {t("onboarding.permissions.microphone.title")}
                </h3>
                <p className="text-sm text-text/60 mb-3">
                  {t("onboarding.permissions.microphone.description")}
                </p>
                {permissions.microphone === "granted" ? (
                  <div className="flex items-center gap-2 text-emerald-400 text-sm">
                    <Check className="w-4 h-4" />
                    {t("onboarding.permissions.granted")}
                  </div>
                ) : permissions.microphone === "waiting" ? (
                  <div className="flex items-center gap-2 text-text/50 text-sm">
                    <Loader2 className="w-4 h-4 animate-spin" />
                    {t("onboarding.permissions.waiting")}
                  </div>
                ) : permissions.microphone === "error" ? (
                  <div className="flex items-center gap-2 text-amber-400 text-sm">
                    <AlertCircle className="w-4 h-4" />
                    {t("onboarding.permissions.errors.checkFailed")}
                  </div>
                ) : (
                  <button
                    onClick={handleGrantMicrophone}
                    className="px-4 py-2 rounded-lg bg-logo-primary hover:bg-logo-primary/90 text-white text-sm font-medium transition-colors"
                  >
                    {isWindows
                      ? t("accessibility.openSettings")
                      : t("onboarding.permissions.grant")}
                  </button>
                )}
              </div>
            </div>
          </div>
        )}

        {/* Accessibility Permission Card */}
        {showAccessibilityPermission && (
          <div className="w-full p-4 rounded-lg bg-white/5 border border-mid-gray/20">
            <div className="flex items-center gap-4">
              <div className="p-3 rounded-full bg-logo-primary/20 shrink-0">
                <Keyboard className="w-6 h-6 text-accent-text" />
              </div>
              <div className="flex-1 min-w-0">
                <h3 className="font-medium text-text">
                  {t("onboarding.permissions.accessibility.title")}
                </h3>
                <p className="text-sm text-text/60 mb-3">
                  {t("onboarding.permissions.accessibility.description")}
                </p>
                {permissions.accessibility === "granted" ? (
                  <div className="flex items-center gap-2 text-emerald-400 text-sm">
                    <Check className="w-4 h-4" />
                    {t("onboarding.permissions.granted")}
                  </div>
                ) : permissions.accessibility === "waiting" ? (
                  <div className="flex items-center gap-2 text-text/50 text-sm">
                    <Loader2 className="w-4 h-4 animate-spin" />
                    {t("onboarding.permissions.waiting")}
                  </div>
                ) : permissions.accessibility === "error" ? (
                  <div className="flex items-center gap-2 text-amber-400 text-sm">
                    <AlertCircle className="w-4 h-4" />
                    {t("onboarding.permissions.errors.checkFailed")}
                  </div>
                ) : (
                  <button
                    onClick={handleGrantAccessibility}
                    className="px-4 py-2 rounded-lg bg-logo-primary hover:bg-logo-primary/90 text-white text-sm font-medium transition-colors"
                  >
                    {t("onboarding.permissions.grant")}
                  </button>
                )}
              </div>
            </div>
          </div>
        )}

        {isMacOS && (
          <button
            onClick={handleRecheckPermissions}
            disabled={isRechecking}
            className="flex items-center gap-2 px-4 py-2 rounded-lg border border-mid-gray/30 text-text/80 hover:bg-white/5 disabled:opacity-50 disabled:cursor-not-allowed text-sm font-medium transition-colors"
          >
            <RefreshCw
              className={`w-4 h-4 ${isRechecking ? "animate-spin" : ""}`}
            />
            {t("onboarding.permissions.recheck")}
          </button>
        )}
      </div>
    </div>
  );
};

export default AccessibilityOnboarding;
