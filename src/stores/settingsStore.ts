import { create } from "zustand";
import { subscribeWithSelector } from "zustand/middleware";
import { listen } from "@tauri-apps/api/event";
import type {
  AppSettings as Settings,
  AudioDevice,
  TranscribeAcceleratorSetting,
  OrtAcceleratorSetting,
  ShortcutActivation,
  VadBackend,
  LogLevel,
  Result as CommandResult,
} from "@/bindings";
import { commands } from "@/bindings";
import { toast } from "sonner";
import i18n from "i18next";

interface SettingsStore {
  settings: Settings | null;
  defaultSettings: Settings | null;
  isLoading: boolean;
  isUpdating: Record<string, boolean>;
  audioDevices: AudioDevice[];
  outputDevices: AudioDevice[];
  customSounds: { start: boolean; stop: boolean };
  postProcessModelOptions: Record<string, string[]>;
  // null until loadUpdateChecksLocked() resolves
  updateChecksLocked: boolean | null;

  // Actions
  initialize: () => Promise<void>;
  loadDefaultSettings: () => Promise<void>;
  loadUpdateChecksLocked: () => Promise<void>;
  updateSetting: <K extends keyof Settings>(
    key: K,
    value: Settings[K],
  ) => Promise<boolean>;
  resetSetting: (key: keyof Settings) => Promise<void>;
  refreshSettings: () => Promise<void>;
  refreshAudioDevices: () => Promise<void>;
  refreshOutputDevices: () => Promise<void>;
  updateBinding: (id: string, binding: string) => Promise<void>;
  resetBinding: (id: string) => Promise<void>;
  getSetting: <K extends keyof Settings>(key: K) => Settings[K] | undefined;
  isUpdatingKey: (key: string) => boolean;
  playTestSound: (soundType: "start" | "stop") => Promise<void>;
  checkCustomSounds: () => Promise<void>;
  setPostProcessProvider: (providerId: string) => Promise<void>;
  updatePostProcessSetting: (
    settingType: "base_url" | "api_key" | "model",
    providerId: string,
    value: string,
  ) => Promise<void>;
  updatePostProcessBaseUrl: (
    providerId: string,
    baseUrl: string,
  ) => Promise<void>;
  updatePostProcessApiKey: (
    providerId: string,
    apiKey: string,
  ) => Promise<void>;
  updatePostProcessModel: (providerId: string, model: string) => Promise<void>;
  fetchPostProcessModels: (providerId: string) => Promise<string[]>;
  setPostProcessModelOptions: (providerId: string, models: string[]) => void;

  // Internal state setters
  setSettings: (settings: Settings | null) => void;
  setDefaultSettings: (defaultSettings: Settings | null) => void;
  setLoading: (loading: boolean) => void;
  setUpdating: (key: string, updating: boolean) => void;
  setAudioDevices: (devices: AudioDevice[]) => void;
  setOutputDevices: (devices: AudioDevice[]) => void;
  setCustomSounds: (sounds: { start: boolean; stop: boolean }) => void;
}

// Note: Default settings are now fetched from Rust via commands.getDefaultSettings()
// This ensures platform-specific defaults (like overlay_position, shortcuts, paste_method) work correctly

const DEFAULT_AUDIO_DEVICE: AudioDevice = {
  index: "default",
  name: "Default",
  is_default: true,
};

const saveFailureKey = "unifiedHistory.feedback.updateFailed";

// Specta 将 Rust Err 包在成功 resolve 的 Promise 内；不能仅靠 catch 判断保存结果。
function savedResult<T>(result: CommandResult<T, string>): T {
  if (result.status === "error") throw new Error(i18n.t(saveFailureKey));
  return result.data;
}

function notifySaveFailure(): Error {
  const error = new Error(i18n.t(saveFailureKey));
  toast.error(error.message);
  // 后端错误可能包含正文、地址或凭据，只记录固定分类。
  console.error("Settings save failed");
  return error;
}

const settingMutations = new Map<
  keyof Settings,
  {
    identity: symbol;
    pending: number;
    baseline: Settings | null;
    confirmedSequence: number;
    latestFailed: boolean;
  }
>();
const settingGenerations = new Map<keyof Settings, number>();

function advanceSetting(key: keyof Settings): number {
  const next = (settingGenerations.get(key) ?? 0) + 1;
  settingGenerations.set(key, next);
  return next;
}

const settingUpdaters: {
  [K in keyof Settings]?: (
    value: Settings[K],
  ) => Promise<CommandResult<null, string>>;
} = {
  always_on_microphone: (value) =>
    commands.updateMicrophoneMode(value as boolean),
  audio_feedback: (value) =>
    commands.changeAudioFeedbackSetting(value as boolean),
  audio_feedback_volume: (value) =>
    commands.changeAudioFeedbackVolumeSetting(value as number),
  sound_theme: (value) => commands.changeSoundThemeSetting(value as string),
  start_hidden: (value) => commands.changeStartHiddenSetting(value as boolean),
  autostart_enabled: (value) =>
    commands.changeAutostartSetting(value as boolean),
  update_checks_enabled: (value) =>
    commands.changeUpdateChecksSetting(value as boolean),
  show_whats_new_on_update: (value) =>
    commands.changeShowWhatsNewOnUpdateSetting(value as boolean),
  whats_new_last_seen_version: (value) =>
    commands.changeWhatsNewLastSeenVersionSetting(value as string),
  shortcut_activation: (value) =>
    commands.changeShortcutActivationSetting(value as ShortcutActivation),
  hold_threshold_ms: (value) =>
    commands.changeHoldThresholdMsSetting(value as number),
  selected_microphone: (value) =>
    commands.setSelectedMicrophone(
      (value as string) === "Default" || value === null
        ? "default"
        : (value as string),
    ),
  selected_channel: (value) => commands.setSelectedChannel(value ?? null),
  clamshell_microphone: (value) =>
    commands.setClamshellMicrophone(
      (value as string) === "Default" ? "default" : (value as string),
    ),
  selected_output_device: (value) =>
    commands.setSelectedOutputDevice(
      (value as string) === "Default" || value === null
        ? "default"
        : (value as string),
    ),
  recording_retention_period: (value) =>
    commands.updateRecordingRetentionPeriod(value as string),
  translate_to_english: (value) =>
    commands.changeTranslateToEnglishSetting(value as boolean),
  selected_language: (value) =>
    commands.changeSelectedLanguageSetting(value as string),
  overlay_position: (value) =>
    commands.changeOverlayPositionSetting(value as string),
  debug_mode: (value) => commands.changeDebugModeSetting(value as boolean),
  custom_words: (value) => commands.updateCustomWords(value as string[]),
  word_correction_threshold: (value) =>
    commands.changeWordCorrectionThresholdSetting(value as number),
  paste_delay_ms: (value) =>
    commands.changePasteDelayMsSetting(value as number),
  paste_delay_after_ms: (value) =>
    commands.changePasteDelayAfterMsSetting(value as number),
  reliable_paste: (value) =>
    commands.changeReliablePasteSetting(value as boolean),
  paste_method: (value) => commands.changePasteMethodSetting(value as string),
  typing_tool: (value) => commands.changeTypingToolSetting(value as string),
  external_script_path: (value) =>
    commands.changeExternalScriptPathSetting(value as string | null),
  clipboard_handling: (value) =>
    commands.changeClipboardHandlingSetting(value as string),
  clipboard_enabled: (value) =>
    commands.changeClipboardEnabledSetting(value as boolean),
  clipboard_max_records: (value) =>
    commands.changeClipboardMaxRecordsSetting(value as number),
  clipboard_hotkey_enabled: (value) =>
    commands.changeClipboardHotkeyEnabledSetting(value as boolean),
  clipboard_hotkey: (value) =>
    commands.changeClipboardHotkeySetting(value as string),
  auto_submit: (value) => commands.changeAutoSubmitSetting(value as boolean),
  auto_submit_key: (value) =>
    commands.changeAutoSubmitKeySetting(value as string),
  history_limit: (value) => commands.updateHistoryLimit(value as number),
  model_unload_timeout: (value) => {
    if (value === undefined)
      return Promise.reject(new Error(i18n.t(saveFailureKey)));
    return commands.setModelUnloadTimeout(value);
  },
  post_process_enabled: (value) =>
    commands.changePostProcessEnabledSetting(value as boolean),
  post_process_selected_prompt_id: (value) =>
    commands.setPostProcessSelectedPrompt(value as string),
  mute_while_recording: (value) =>
    commands.changeMuteWhileRecordingSetting(value as boolean),
  append_trailing_space: (value) =>
    commands.changeAppendTrailingSpaceSetting(value as boolean),
  log_level: (value) => commands.setLogLevel(value as LogLevel),
  app_language: (value) => commands.changeAppLanguageSetting(value as string),
  theme: (value) => commands.changeThemeSetting(value as string),
  experimental_enabled: (value) =>
    commands.changeExperimentalEnabledSetting(value as boolean),
  lazy_stream_close: (value) =>
    commands.changeLazyStreamCloseSetting(value as boolean),
  overlay_style: (value) => commands.changeOverlayStyleSetting(value as string),
  vad_enabled: (value) => commands.changeVadEnabledSetting(value as boolean),
  vad_backend: (value) => commands.changeVadBackendSetting(value as VadBackend),
  filler_word_removal_enabled: (value) =>
    commands.changeFillerWordRemovalEnabledSetting(value as boolean),
  show_tray_icon: (value) =>
    commands.changeShowTrayIconSetting(value as boolean),
  transcribe_accelerator: (value) =>
    commands.changeTranscribeAcceleratorSetting(
      value as TranscribeAcceleratorSetting,
    ),
  ort_accelerator: (value) =>
    commands.changeOrtAcceleratorSetting(value as OrtAcceleratorSetting),
  transcribe_gpu_device: (value) =>
    commands.changeTranscribeGpuDevice(value as string | null),
  extra_recording_buffer_ms: (value) =>
    commands.changeExtraRecordingBufferSetting(value as number),
};

export const useSettingsStore = create<SettingsStore>()(
  subscribeWithSelector((set, get) => ({
    settings: null,
    defaultSettings: null,
    isLoading: true,
    isUpdating: {},
    audioDevices: [],
    outputDevices: [],
    customSounds: { start: false, stop: false },
    postProcessModelOptions: {},
    updateChecksLocked: null,

    // Internal setters
    setSettings: (settings) => set({ settings }),
    setDefaultSettings: (defaultSettings) => set({ defaultSettings }),
    setLoading: (isLoading) => set({ isLoading }),
    setUpdating: (key, updating) =>
      set((state) => ({
        isUpdating: { ...state.isUpdating, [key]: updating },
      })),
    setAudioDevices: (audioDevices) => set({ audioDevices }),
    setOutputDevices: (outputDevices) => set({ outputDevices }),
    setCustomSounds: (customSounds) => set({ customSounds }),

    // Getters
    getSetting: (key) => get().settings?.[key],
    isUpdatingKey: (key) => get().isUpdating[key] || false,

    // Load settings from store
    refreshSettings: async () => {
      const generations = new Map(settingGenerations);
      const pendingAtRead = new Set(settingMutations.keys());
      try {
        const result = await commands.getAppSettings();
        if (result.status === "ok") {
          const settings = result.data;
          const normalizedSettings: Settings = {
            ...settings,
            always_on_microphone: settings.always_on_microphone ?? false,
            selected_microphone: settings.selected_microphone ?? "Default",
            clamshell_microphone: settings.clamshell_microphone ?? "Default",
            selected_output_device:
              settings.selected_output_device ?? "Default",
          };
          set((state) => {
            let merged = normalizedSettings;
            if (state.settings) {
              for (const key of Object.keys(
                state.settings,
              ) as (keyof Settings)[]) {
                if (
                  pendingAtRead.has(key) ||
                  settingMutations.has(key) ||
                  generations.get(key) !== settingGenerations.get(key)
                ) {
                  merged = { ...merged, [key]: state.settings[key] };
                }
              }
            }
            return { settings: merged, isLoading: false };
          });
        } else {
          console.error("Failed to load settings");
          set({ isLoading: false });
        }
      } catch (error) {
        console.error("Failed to load settings");
        set({ isLoading: false });
      }
    },

    // Load audio devices
    refreshAudioDevices: async () => {
      try {
        const result = await commands.getAvailableMicrophones();
        if (result.status === "ok") {
          const devicesWithDefault = [
            DEFAULT_AUDIO_DEVICE,
            ...result.data.filter(
              (d) => d.name !== "Default" && d.name !== "default",
            ),
          ];
          set({ audioDevices: devicesWithDefault });
        } else {
          set({ audioDevices: [DEFAULT_AUDIO_DEVICE] });
        }
      } catch (error) {
        console.error("Failed to load audio devices:", error);
        set({ audioDevices: [DEFAULT_AUDIO_DEVICE] });
      }
    },

    // Load output devices
    refreshOutputDevices: async () => {
      try {
        const result = await commands.getAvailableOutputDevices();
        if (result.status === "ok") {
          const devicesWithDefault = [
            DEFAULT_AUDIO_DEVICE,
            ...result.data.filter(
              (d) => d.name !== "Default" && d.name !== "default",
            ),
          ];
          set({ outputDevices: devicesWithDefault });
        } else {
          set({ outputDevices: [DEFAULT_AUDIO_DEVICE] });
        }
      } catch (error) {
        console.error("Failed to load output devices:", error);
        set({ outputDevices: [DEFAULT_AUDIO_DEVICE] });
      }
    },

    // Play a test sound
    playTestSound: async (soundType: "start" | "stop") => {
      try {
        await commands.playTestSound(soundType);
      } catch (error) {
        console.error(`Failed to play test sound (${soundType}):`, error);
      }
    },

    checkCustomSounds: async () => {
      try {
        const sounds = await commands.checkCustomSounds();
        get().setCustomSounds(sounds);
      } catch (error) {
        console.error("Failed to check custom sounds:", error);
      }
    },

    // Update a specific setting
    updateSetting: async <K extends keyof Settings>(
      key: K,
      value: Settings[K],
    ) => {
      const { settings, setUpdating } = get();
      const previous = settingMutations.get(key);
      const identity = Symbol();
      const sequence = advanceSetting(key);
      const baseline = previous?.baseline ?? settings;
      settingMutations.set(key, {
        identity,
        pending: (previous?.pending ?? 0) + 1,
        baseline,
        confirmedSequence: previous?.confirmedSequence ?? 0,
        latestFailed: false,
      });
      const current = () => settingMutations.get(key)?.identity === identity;
      setUpdating(String(key), true);

      try {
        const updater = settingUpdaters[key];
        if (!updater || !settings) throw new Error(i18n.t(saveFailureKey));
        set((state) => ({
          settings: state.settings ? { ...state.settings, [key]: value } : null,
        }));
        savedResult(await updater(value));
        const group = settingMutations.get(key);
        if (group && sequence > group.confirmedSequence) {
          group.confirmedSequence = sequence;
          group.baseline = group.baseline
            ? { ...group.baseline, [key]: value }
            : null;
          if (group.latestFailed) {
            set((state) => ({
              settings: state.settings
                ? { ...state.settings, [key]: value }
                : null,
            }));
          }
        }
        return true;
      } catch {
        if (current()) {
          const group = settingMutations.get(key);
          if (group) group.latestFailed = true;
        }
        // 保存失败仅核对本字段。读取不会重发原动作，也不能覆盖期间其他字段的乐观修改。
        let rollback = (settingMutations.get(key)?.baseline ?? baseline)?.[key];
        if (current()) {
          const confirmedAtRead = settingMutations.get(key)?.confirmedSequence;
          try {
            const result = await commands.getAppSettings();
            rollback =
              result.status === "ok" &&
              confirmedAtRead === settingMutations.get(key)?.confirmedSequence
                ? result.data[key]
                : (settingMutations.get(key)?.baseline ?? baseline)?.[key];
          } catch {
            // 无法确认最新值时保留本轮开始前的已知基准，不使用另一在途修改作基准。
            rollback = (settingMutations.get(key)?.baseline ?? baseline)?.[key];
          }
          set((state) => ({
            settings:
              current() &&
              state.settings &&
              Object.is(state.settings[key], value)
                ? { ...state.settings, [key]: rollback }
                : state.settings,
          }));
        }
        notifySaveFailure();
        return false;
      } finally {
        advanceSetting(key);
        const pending = settingMutations.get(key);
        if (pending && --pending.pending === 0) {
          settingMutations.delete(key);
          setUpdating(String(key), false);
        }
      }
    },

    // Reset a setting to its default value
    resetSetting: async (key) => {
      const { defaultSettings } = get();
      if (defaultSettings) {
        const defaultValue = defaultSettings[key];
        if (defaultValue !== undefined) {
          await get().updateSetting(key, defaultValue);
        }
      }
    },

    // Update a specific binding
    updateBinding: async (id, binding) => {
      const { settings, setUpdating } = get();
      const updateKey = `binding_${id}`;
      const originalBinding = settings?.bindings?.[id]?.current_binding;

      setUpdating(updateKey, true);

      try {
        // Optimistic update
        set((state) => ({
          settings: state.settings
            ? {
                ...state.settings,
                bindings: {
                  ...state.settings.bindings,
                  [id]: {
                    ...state.settings.bindings?.[id]!,
                    current_binding: binding,
                  },
                },
              }
            : null,
        }));

        const result = await commands.changeBinding(id, binding);

        // Check if the command executed successfully
        if (result.status === "error") {
          throw new Error(i18n.t(saveFailureKey));
        }

        // Check if the binding change was successful
        if (!result.data.success) {
          throw new Error(i18n.t(saveFailureKey));
        }
      } catch {
        const error = notifySaveFailure();

        // Rollback on error
        if (originalBinding && get().settings) {
          set((state) => ({
            settings: state.settings
              ? {
                  ...state.settings,
                  bindings: {
                    ...state.settings.bindings,
                    [id]: {
                      ...state.settings.bindings?.[id]!,
                      current_binding: originalBinding,
                    },
                  },
                }
              : null,
          }));
        }

        // Re-throw to let the caller know it failed
        throw error;
      } finally {
        setUpdating(updateKey, false);
      }
    },

    // Reset a specific binding
    resetBinding: async (id) => {
      const { setUpdating, refreshSettings } = get();
      const updateKey = `binding_${id}`;

      setUpdating(updateKey, true);

      try {
        const result = savedResult(await commands.resetBinding(id));
        if (!result.success) throw new Error(i18n.t(saveFailureKey));
        await refreshSettings();
      } catch {
        notifySaveFailure();
      } finally {
        setUpdating(updateKey, false);
      }
    },

    setPostProcessProvider: async (providerId) => {
      const {
        settings,
        setUpdating,
        refreshSettings,
        setPostProcessModelOptions,
      } = get();
      const updateKey = "post_process_provider_id";
      const previousId = settings?.post_process_provider_id ?? null;

      setUpdating(updateKey, true);

      if (settings) {
        set((state) => ({
          settings: state.settings
            ? { ...state.settings, post_process_provider_id: providerId }
            : null,
        }));
      }

      // Clear cached model options for the new provider so the dropdown
      // doesn't show stale models from a previous fetch or base_url.
      setPostProcessModelOptions(providerId, []);

      try {
        savedResult(await commands.setPostProcessProvider(providerId));
        await refreshSettings();
      } catch {
        notifySaveFailure();
        if (previousId !== null) {
          set((state) => ({
            settings: state.settings
              ? { ...state.settings, post_process_provider_id: previousId }
              : null,
          }));
        }
      } finally {
        setUpdating(updateKey, false);
      }
    },

    // Generic updater for post-processing provider settings
    updatePostProcessSetting: async (
      settingType: "base_url" | "api_key" | "model",
      providerId: string,
      value: string,
    ) => {
      const { setUpdating, refreshSettings } = get();
      const updateKey = `post_process_${settingType}:${providerId}`;

      setUpdating(updateKey, true);

      try {
        if (settingType === "base_url") {
          savedResult(
            await commands.changePostProcessBaseUrlSetting(providerId, value),
          );
        } else if (settingType === "api_key") {
          savedResult(
            await commands.changePostProcessApiKeySetting(providerId, value),
          );
        } else if (settingType === "model") {
          savedResult(
            await commands.changePostProcessModelSetting(providerId, value),
          );
        }
        await refreshSettings();
      } catch {
        notifySaveFailure();
      } finally {
        setUpdating(updateKey, false);
      }
    },

    updatePostProcessBaseUrl: async (providerId, baseUrl) => {
      const { setUpdating, refreshSettings } = get();
      const updateKey = `post_process_base_url:${providerId}`;

      setUpdating(updateKey, true);

      try {
        // Persist the new base URL first.
        const urlResult = await commands.changePostProcessBaseUrlSetting(
          providerId,
          baseUrl,
        );
        savedResult(urlResult);

        // Reset the stored model since the previous value is almost certainly
        // invalid for the new endpoint (e.g. switching Custom from Groq to
        // Cerebras). Only proceed if the reset succeeds.
        const modelResult = await commands.changePostProcessModelSetting(
          providerId,
          "",
        );
        savedResult(modelResult);

        // Clear cached model options only after both backend writes succeed.
        set((state) => ({
          postProcessModelOptions: {
            ...state.postProcessModelOptions,
            [providerId]: [],
          },
        }));

        // Single refresh after both backend writes.
        await refreshSettings();
      } catch {
        notifySaveFailure();
        // 第一次保存可能已成功；这里只重新读取，不自动重做第二个保存动作。
        await refreshSettings();
      } finally {
        setUpdating(updateKey, false);
      }
    },

    updatePostProcessApiKey: async (providerId, apiKey) => {
      // Clear cached models when API key changes - user should click refresh after
      set((state) => ({
        postProcessModelOptions: {
          ...state.postProcessModelOptions,
          [providerId]: [],
        },
      }));
      return get().updatePostProcessSetting("api_key", providerId, apiKey);
    },

    updatePostProcessModel: async (providerId, model) => {
      return get().updatePostProcessSetting("model", providerId, model);
    },

    fetchPostProcessModels: async (providerId) => {
      const updateKey = `post_process_models_fetch:${providerId}`;
      const { setUpdating, setPostProcessModelOptions } = get();

      setUpdating(updateKey, true);

      try {
        // Call Tauri backend command instead of fetch
        const result = await commands.fetchPostProcessModels(providerId);
        if (result.status === "ok") {
          setPostProcessModelOptions(providerId, result.data);
          return result.data;
        } else {
          console.error("Failed to fetch models");
          return [];
        }
      } catch (error) {
        console.error("Failed to fetch models");
        // Don't cache empty array on error - let user retry
        return [];
      } finally {
        setUpdating(updateKey, false);
      }
    },

    setPostProcessModelOptions: (providerId, models) =>
      set((state) => ({
        postProcessModelOptions: {
          ...state.postProcessModelOptions,
          [providerId]: models,
        },
      })),

    // Load default settings from Rust
    loadDefaultSettings: async () => {
      try {
        const result = await commands.getDefaultSettings();
        if (result.status === "ok") {
          set({ defaultSettings: result.data });
        } else {
          console.error("Failed to load default settings");
        }
      } catch (error) {
        console.error("Failed to load default settings");
      }
    },

    // Check whether update checks are locked by system configuration
    // (e.g. HANDY_DISABLE_UPDATER, set by the Nix package)
    loadUpdateChecksLocked: async () => {
      try {
        const locked = await commands.isUpdateChecksLocked();
        set({ updateChecksLocked: locked });
      } catch (error) {
        console.error("Failed to check update checks lock state:", error);
        // Fail open: an unknown lock state means "not locked", otherwise the
        // update checker waits for it forever and checks never start.
        set({ updateChecksLocked: false });
      }
    },

    // Initialize everything
    initialize: async () => {
      const {
        refreshSettings,
        checkCustomSounds,
        loadDefaultSettings,
        loadUpdateChecksLocked,
      } = get();

      // Note: Audio devices are NOT refreshed here. The frontend (App.tsx)
      // is responsible for calling refreshAudioDevices/refreshOutputDevices
      // after onboarding completes. This avoids triggering permission dialogs
      // on macOS before the user is ready.
      await Promise.all([
        loadDefaultSettings(),
        refreshSettings(),
        checkCustomSounds(),
        loadUpdateChecksLocked(),
      ]);

      // Re-fetch settings when the backend changes them (e.g. language
      // reset during model switch). The backend is the source of truth.
      listen("model-state-changed", () => {
        get().refreshSettings();
      });
      listen("settings-save-failed", () => {
        notifySaveFailure();
      });
      listen<{ setting?: string }>("settings-changed", (event) => {
        get().refreshSettings();
        if (event.payload.setting === "selected_microphone") {
          get().refreshAudioDevices();
        }
      });
    },
  })),
);
