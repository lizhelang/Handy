import React, { useEffect, useState, useRef, useCallback } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { formatKeyCombination } from "../../lib/utils/keyboard";
import { ResetButton } from "../ui/ResetButton";
import { SettingContainer } from "../ui/SettingContainer";
import { useSettings } from "../../hooks/useSettings";
import { useOsType } from "../../hooks/useOsType";
import { useShortcutCapture } from "../../hooks/useShortcutCapture";
import { toast } from "sonner";
import { openUrl } from "@tauri-apps/plugin-opener";
import { SECURE_INPUT_HELP_URL } from "../SecureInputWarning";

interface HandyKeysShortcutInputProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
  shortcutId: string;
  disabled?: boolean;
}

interface HandyKeysEvent {
  capture_token: string;
  modifiers: string[];
  key: string | null;
  is_key_down: boolean;
  hotkey_string: string;
}

export const HandyKeysShortcutInput: React.FC<HandyKeysShortcutInputProps> = ({
  descriptionMode = "tooltip",
  grouped = false,
  shortcutId,
  disabled = false,
}) => {
  const { t } = useTranslation();
  const { getSetting, updateBinding, resetBinding, isUpdating, isLoading } =
    useSettings();
  const [isRecording, setIsRecording] = useState(false);
  const [currentKeys, setCurrentKeys] = useState<string>("");
  const committing = useRef(false);
  const editGeneration = useRef(0);
  const capture = useShortcutCapture(
    true,
    () => {
      toast.error(t("secureInput.recorderBlocked"), {
        action: {
          label: t("secureInput.learnMore"),
          onClick: () => openUrl(SECURE_INPUT_HELP_URL),
        },
      });
    },
    () => {
      editGeneration.current += 1;
      setIsRecording(false);
      setCurrentKeys("");
      currentKeysRef.current = "";
      keyedShortcutRef.current = "";
      modifierOnlyShortcutRef.current = "";
    },
  );
  const shortcutRef = useRef<HTMLDivElement | null>(null);
  const unlistenRef = useRef<(() => void) | null>(null);
  // Use a ref to track currentKeys for the event handler (avoids stale closure)
  const currentKeysRef = useRef<string>("");
  // Track keyed vs modifier-only captures separately so a combo commits only
  // on its key's release and a modifier-only shortcut only once every
  // modifier is released. Committing on the *first* release (the old
  // behavior) silently saved just the modifier whenever the key event never
  // arrived — e.g. while macOS Secure Input is active (issue #1578).
  const keyedShortcutRef = useRef<string>("");
  const modifierOnlyShortcutRef = useRef<string>("");
  const osType = useOsType();

  const bindings = getSetting("bindings") || {};

  // Handle cancellation
  const cancelRecording = useCallback(async () => {
    if (!isRecording) return;

    editGeneration.current += 1;
    if (!(await capture.end()).released) return;

    // Stop listening only after this lease has actually ended.
    if (unlistenRef.current) {
      unlistenRef.current();
      unlistenRef.current = null;
    }

    setIsRecording(false);
    setCurrentKeys("");
    currentKeysRef.current = "";
    keyedShortcutRef.current = "";
    modifierOnlyShortcutRef.current = "";
  }, [isRecording, capture.end]);

  // Set up event listener for handy-keys events
  useEffect(() => {
    if (!isRecording) return;

    let cleanup = false;

    const setupListener = async () => {
      // Listen for key events from backend
      const commitAndStop = async (keysToCommit: string) => {
        if (committing.current) return;
        committing.current = true;
        const attempt = editGeneration.current;
        const end = await capture.end();
        if (!end.released || attempt !== editGeneration.current) {
          committing.current = false;
          return;
        }
        try {
          if (end.canSave) await updateBinding(shortcutId, keysToCommit);
        } catch {
          /* store 已显示通用失败，不自动提交旧设置。 */
        }
        committing.current = false;

        // Stop recording
        if (unlistenRef.current) {
          unlistenRef.current();
          unlistenRef.current = null;
        }
        setIsRecording(false);
        setCurrentKeys("");
        currentKeysRef.current = "";
        keyedShortcutRef.current = "";
        modifierOnlyShortcutRef.current = "";
      };

      const unlisten = await listen<HandyKeysEvent>(
        "handy-keys-event",
        async (event) => {
          if (cleanup || !capture.owns(event.payload.capture_token)) return;

          const { hotkey_string, is_key_down, key, modifiers } = event.payload;

          if (is_key_down && hotkey_string) {
            // Update both state (for display) and refs (for release handler)
            if (key) {
              keyedShortcutRef.current = hotkey_string;
            } else {
              modifierOnlyShortcutRef.current = hotkey_string;
            }
            currentKeysRef.current = hotkey_string;
            setCurrentKeys(hotkey_string);
          } else if (!is_key_down && key) {
            // The main key was released — commit the keyed combo. The release
            // event's hotkey_string still contains the key, so it works even
            // if the key-down was somehow missed. Never fall back to a
            // modifier-only capture here: that's how bindings used to get
            // silently overwritten with just the modifier (issue #1578).
            const keysToCommit = keyedShortcutRef.current || hotkey_string;
            if (keysToCommit) {
              await commitAndStop(keysToCommit);
            }
          } else if (
            !is_key_down &&
            !key &&
            modifiers.length === 0 &&
            !keyedShortcutRef.current &&
            modifierOnlyShortcutRef.current
          ) {
            // Every modifier released without a main key ever going down —
            // commit as a modifier-only shortcut
            await commitAndStop(modifierOnlyShortcutRef.current);
          }
        },
      );

      if (cleanup) unlisten();
      else unlistenRef.current = unlisten;
    };

    setupListener();

    return () => {
      cleanup = true;
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
      // token 的卸载清理由 useShortcutCapture 负责；listener 重建不能结束别人的录制。
    };
  }, [
    isRecording,
    shortcutId,
    updateBinding,
    cancelRecording,
    capture.end,
    capture.owns,
    t,
  ]);

  // Handle click outside
  useEffect(() => {
    if (!isRecording) return;

    const handleClickOutside = (e: MouseEvent) => {
      if (
        shortcutRef.current &&
        !shortcutRef.current.contains(e.target as Node)
      ) {
        cancelRecording();
      }
    };

    window.addEventListener("click", handleClickOutside);
    return () => window.removeEventListener("click", handleClickOutside);
  }, [isRecording, cancelRecording]);

  // Start recording a new shortcut
  const startRecording = async () => {
    if (isRecording) return;

    if (!(await capture.begin(shortcutId))) return;
    editGeneration.current += 1;
    setIsRecording(true);
    setCurrentKeys("");
    currentKeysRef.current = "";
    keyedShortcutRef.current = "";
    modifierOnlyShortcutRef.current = "";
  };

  // Format the current shortcut keys being recorded
  const formatCurrentKeys = (): string => {
    if (!currentKeys) return t("settings.general.shortcut.pressKeys");
    return formatKeyCombination(currentKeys, osType);
  };

  // If still loading, show loading state
  if (isLoading) {
    return (
      <SettingContainer
        title={t("settings.general.shortcut.title")}
        description={t("settings.general.shortcut.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="text-sm text-mid-gray">
          {t("settings.general.shortcut.loading")}
        </div>
      </SettingContainer>
    );
  }

  // If no bindings are loaded, show empty state
  if (Object.keys(bindings).length === 0) {
    return (
      <SettingContainer
        title={t("settings.general.shortcut.title")}
        description={t("settings.general.shortcut.description")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="text-sm text-mid-gray">
          {t("settings.general.shortcut.none")}
        </div>
      </SettingContainer>
    );
  }

  const binding = bindings[shortcutId];
  if (!binding) {
    return (
      <SettingContainer
        title={t("settings.general.shortcut.title")}
        description={t("settings.general.shortcut.notFound")}
        descriptionMode={descriptionMode}
        grouped={grouped}
      >
        <div className="text-sm text-mid-gray">
          {t("settings.general.shortcut.none")}
        </div>
      </SettingContainer>
    );
  }

  // Get translated name and description for the binding
  const translatedName = t(
    `settings.general.shortcut.bindings.${shortcutId}.name`,
    binding.name,
  );
  const translatedDescription = t(
    `settings.general.shortcut.bindings.${shortcutId}.description`,
    binding.description,
  );

  return (
    <SettingContainer
      title={translatedName}
      description={translatedDescription}
      descriptionMode={descriptionMode}
      grouped={grouped}
      disabled={disabled}
      layout="horizontal"
    >
      <div className="flex items-center space-x-1">
        {isRecording ? (
          <div
            ref={shortcutRef}
            className="px-2 py-1 text-sm font-semibold border border-logo-primary bg-logo-primary/30 rounded-md"
          >
            {formatCurrentKeys()}
          </div>
        ) : (
          <div
            className="px-2 py-1 text-sm font-semibold bg-mid-gray/10 border border-mid-gray/80 hover:bg-logo-primary/10 rounded-md cursor-pointer hover:border-logo-primary"
            onClick={startRecording}
          >
            {formatKeyCombination(binding.current_binding, osType)}
          </div>
        )}
        <ResetButton
          onClick={() => resetBinding(shortcutId)}
          disabled={isRecording || isUpdating(`binding_${shortcutId}`)}
        />
      </div>
    </SettingContainer>
  );
};
