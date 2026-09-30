#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
use crate::apple_intelligence;
use crate::audio_feedback::{play_feedback_sound, play_feedback_sound_blocking, SoundType};
use crate::audio_toolkit::{is_microphone_access_denied, is_no_input_device_error, VadPolicy};
use crate::managers::audio::AudioRecordingManager;
use crate::managers::history::HistoryManager;
use crate::managers::model::ModelManager;
use crate::managers::transcription::StreamWorkKind;
use crate::managers::transcription::TranscriptionManager;
use crate::settings::{
    get_settings, AppSettings, OverlayStyle, APPLE_INTELLIGENCE_PROVIDER_ID,
    LOCAL_POST_PROCESS_PROVIDER_ID,
};
use crate::shortcut;
use crate::tray::{set_tray_state, TrayIconState};
use crate::utils::{
    self, show_processing_overlay, show_recording_overlay, show_transcribing_overlay,
};
use crate::TranscriptionCoordinator;
use ferrous_opencc::{config::BuiltinConfig, OpenCC};
use inputia_handy_runtime::decision::{
    DecisionLimits, DecisionQuestion, DecisionRequest, DecisionResponse,
};
use log::{debug, error, warn};
use once_cell::sync::Lazy;
use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::Manager;
use tauri::{AppHandle, Emitter};

const CANCELLATION_POLL_INTERVAL: Duration = Duration::from_millis(25);

async fn prepare_owned_voice_result(
    app: &AppHandle,
    start: inputia_handy_runtime::voice_protocol::VoiceRequest,
    history_id: i64,
    text: String,
    permission_epoch: u64,
) -> Result<(), String> {
    let service = app
        .try_state::<Arc<crate::managers::integration::IntegrationManager>>()
        .ok_or("统一历史服务不可用")?
        .service
        .clone();
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::input_permission::check_epoch(permission_epoch)?;
        let output = service.prepare_saved_voice_result(start.clone(), history_id, text)?;
        crate::input_permission::check_epoch(permission_epoch)?;
        let coordinator = task_app
            .try_state::<TranscriptionCoordinator>()
            .ok_or("语音协调器不可用")?;
        let view = coordinator
            .notify_voice_result_prepared(
                start.clone(),
                output.intent.item_id,
                output.intent.operation_id,
            )
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "语音结果状态回执未知")??;
        service.project_voice_session(start.client_instance, start.server_instance, view)?;
        Ok(())
    })
    .await
    .map_err(|_| "语音结果准备任务中断".to_owned())?
}

#[derive(Clone, serde::Serialize)]
struct RecordingErrorEvent {
    error_type: String,
    detail: Option<String>,
}

/// Drop guard that notifies the [`TranscriptionCoordinator`] when the
/// transcription pipeline finishes — whether it completes normally or panics.
struct FinishGuard(AppHandle);
impl Drop for FinishGuard {
    fn drop(&mut self) {
        if let Some(c) = self.0.try_state::<TranscriptionCoordinator>() {
            c.notify_processing_finished();
        }
        // The pipeline just freed its large transient buffers (captured PCM,
        // WAV copy, engine scratch); hand the cached pages back to the OS so
        // they don't sit in malloc arenas until they get swapped out (#1792).
        crate::memory::trim_freed_memory();
    }
}

// Shortcut Action Trait
pub trait ShortcutAction: Send + Sync {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str);
}

// Transcribe Action
struct TranscribeAction {
    post_process: bool,
}

/// Field name for structured output JSON schema
const TRANSCRIPTION_FIELD: &str = "transcription";

/// Strip invisible Unicode characters that some LLMs may insert
fn strip_invisible_chars(s: &str) -> String {
    s.replace(['\u{200B}', '\u{200C}', '\u{200D}', '\u{FEFF}'], "")
}

/// Strip a leading `<think>...</think>` block. Some endpoints can't disable
/// reasoning, and some local servers put the reasoning text into `content`
/// instead of a separate field — without this the user would get the model's
/// chain of thought pasted along with the cleaned transcription.
fn strip_think_block(s: &str) -> &str {
    if let Some(rest) = s.trim_start().strip_prefix("<think>") {
        if let Some(end) = rest.find("</think>") {
            return rest[end + "</think>".len()..].trim_start();
        }
    }
    s
}

/// Build a system prompt from the user's prompt template.
/// Removes `${output}` placeholder since the transcription is sent as the user message.
fn format_custom_words_for_prompt(custom_words: &[String]) -> String {
    let words = custom_words
        .iter()
        .map(|word| word.trim())
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    serde_json::to_string(&words).unwrap_or_else(|_| "[]".to_string())
}

fn render_prompt_template(
    prompt_template: &str,
    transcription: &str,
    custom_words: &[String],
) -> String {
    prompt_template.replace("${output}", transcription).replace(
        "${custom_words}",
        &format_custom_words_for_prompt(custom_words),
    )
}

fn build_system_prompt(prompt_template: &str, custom_words: &[String]) -> String {
    render_prompt_template(prompt_template, "", custom_words)
        .trim()
        .to_string()
}

/// Returns `true` when a transcription has no meaningful content to
/// post-process (empty or whitespace-only). Used to skip the post-processing
/// LLM call when nothing was actually transcribed, which would otherwise make
/// the model reply with an error message such as "you need to provide the
/// transcription".
fn is_blank_transcription(transcription: &str) -> bool {
    transcription.trim().is_empty()
}

async fn complete_unless_cancelled<F, C>(operation: F, is_cancelled: C) -> Option<F::Output>
where
    F: Future,
    C: Fn() -> bool,
{
    tokio::pin!(operation);

    loop {
        if is_cancelled() {
            return None;
        }

        if let Ok(result) =
            tokio::time::timeout(CANCELLATION_POLL_INTERVAL, operation.as_mut()).await
        {
            return Some(result);
        }
    }
}

fn should_use_streaming_overlay(style: OverlayStyle, is_streaming: bool) -> bool {
    style == OverlayStyle::Live && is_streaming
}

async fn post_process_transcription(
    app: &AppHandle,
    settings: &AppSettings,
    transcription: &str,
) -> Option<String> {
    if is_blank_transcription(transcription) {
        debug!("Post-processing skipped because the transcription is empty");
        return None;
    }

    let provider = match settings.active_post_process_provider().cloned() {
        Some(provider) => provider,
        None => {
            debug!("Post-processing enabled but no provider is selected");
            return None;
        }
    };

    if provider.id == LOCAL_POST_PROCESS_PROVIDER_ID {
        return crate::custom_words_model::correct_custom_words(app, settings, transcription).await;
    }

    let model = settings
        .post_process_models
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    if model.trim().is_empty() {
        debug!(
            "Post-processing skipped because provider '{}' has no model configured",
            provider.id
        );
        return None;
    }

    let selected_prompt_id = match &settings.post_process_selected_prompt_id {
        Some(id) => id.clone(),
        None => {
            debug!("Post-processing skipped because no prompt is selected");
            return None;
        }
    };

    let prompt = match settings
        .post_process_prompts
        .iter()
        .find(|prompt| prompt.id == selected_prompt_id)
    {
        Some(prompt) => prompt.prompt.clone(),
        None => {
            debug!(
                "Post-processing skipped because prompt '{}' was not found",
                selected_prompt_id
            );
            return None;
        }
    };

    if prompt.trim().is_empty() {
        debug!("Post-processing skipped because the selected prompt is empty");
        return None;
    }

    debug!(
        "Starting LLM post-processing with provider '{}' (model: {})",
        provider.id, model
    );

    let api_key = settings
        .post_process_api_keys
        .get(&provider.id)
        .cloned()
        .unwrap_or_default();

    // Ask these providers to skip reasoning/thinking — post-processing rarely
    // benefits from it and it adds seconds of latency. llm_client picks the
    // field the endpoint understands and retries without it if rejected.
    let disable_reasoning = matches!(provider.id.as_str(), "custom" | "openrouter");

    if provider.supports_structured_output {
        debug!("Using structured outputs for provider '{}'", provider.id);

        let system_prompt = build_system_prompt(&prompt, &settings.custom_words);
        let user_content = transcription.to_string();

        // Handle Apple Intelligence separately since it uses native Swift APIs
        if provider.id == APPLE_INTELLIGENCE_PROVIDER_ID {
            #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
            {
                if !apple_intelligence::check_apple_intelligence_availability() {
                    debug!(
                        "Apple Intelligence selected but not currently available on this device"
                    );
                    return None;
                }

                let token_limit = model.trim().parse::<i32>().unwrap_or(0);
                return match apple_intelligence::process_text_with_system_prompt(
                    &system_prompt,
                    &user_content,
                    token_limit,
                ) {
                    Ok(result) => {
                        if result.trim().is_empty() {
                            debug!("Apple Intelligence returned an empty response");
                            None
                        } else {
                            let result = strip_invisible_chars(&result);
                            debug!(
                                "Apple Intelligence post-processing succeeded. Output length: {} chars",
                                result.len()
                            );
                            Some(result)
                        }
                    }
                    Err(err) => {
                        error!("Apple Intelligence post-processing failed: {}", err);
                        None
                    }
                };
            }

            #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
            {
                debug!("Apple Intelligence provider selected on unsupported platform");
                return None;
            }
        }

        // Define JSON schema for transcription output
        let json_schema = serde_json::json!({
            "type": "object",
            "properties": {
                (TRANSCRIPTION_FIELD): {
                    "type": "string",
                    "description": "The cleaned and processed transcription text"
                }
            },
            "required": [TRANSCRIPTION_FIELD],
            "additionalProperties": false
        });

        match crate::llm_client::send_chat_completion_with_schema(
            &provider,
            api_key.clone(),
            &model,
            user_content,
            Some(system_prompt),
            Some(json_schema),
            disable_reasoning,
        )
        .await
        {
            Ok(Some(content)) => {
                // Parse the JSON response to extract the transcription field
                let content = strip_think_block(&content);
                match serde_json::from_str::<serde_json::Value>(content) {
                    Ok(json) => {
                        if let Some(transcription_value) =
                            json.get(TRANSCRIPTION_FIELD).and_then(|t| t.as_str())
                        {
                            let result = strip_invisible_chars(transcription_value);
                            debug!(
                                "Structured output post-processing succeeded for provider '{}'. Output length: {} chars",
                                provider.id,
                                result.len()
                            );
                            return Some(result);
                        } else {
                            error!("Structured output response missing 'transcription' field");
                            return Some(strip_invisible_chars(content));
                        }
                    }
                    Err(e) => {
                        error!(
                            "Failed to parse structured output JSON: {}. Returning raw content.",
                            e
                        );
                        return Some(strip_invisible_chars(content));
                    }
                }
            }
            Ok(None) => {
                error!("LLM API response has no content");
                return None;
            }
            Err(e) => {
                warn!(
                    "Structured output failed for provider '{}': {}. Falling back to legacy mode.",
                    provider.id, e
                );
                // Fall through to legacy mode below
            }
        }
    }

    // Legacy mode: Replace ${output} variable in the prompt with the actual text
    let processed_prompt = render_prompt_template(&prompt, transcription, &settings.custom_words);
    debug!("Processed prompt length: {} chars", processed_prompt.len());

    match crate::llm_client::send_chat_completion(
        &provider,
        api_key,
        &model,
        processed_prompt,
        disable_reasoning,
    )
    .await
    {
        Ok(Some(content)) => {
            let content = strip_invisible_chars(strip_think_block(&content));
            debug!(
                "LLM post-processing succeeded for provider '{}'. Output length: {} chars",
                provider.id,
                content.len()
            );
            Some(content)
        }
        Ok(None) => {
            error!("LLM API response has no content");
            None
        }
        Err(e) => {
            error!(
                "LLM post-processing failed for provider '{}': {}. Falling back to original transcription.",
                provider.id,
                e
            );
            None
        }
    }
}

async fn maybe_convert_chinese_variant(
    effective_language: &str,
    transcription: &str,
) -> Option<String> {
    // Gate on the language the model actually transcribed in (the effective
    // language), not the persisted intent. A leftover zh-Hans/zh-Hant intent
    // from a previously selected model must not run OpenCC S2T/T2S over output a
    // non-Chinese model produced — that would silently rewrite any shared CJK
    // characters (e.g. Japanese kanji) in the result.
    let is_simplified = effective_language == "zh-Hans";
    let is_traditional = effective_language == "zh-Hant";

    if !is_simplified && !is_traditional {
        debug!("effective language is not Simplified or Traditional Chinese; skipping conversion");
        return None;
    }

    debug!(
        "Starting Chinese variant conversion using OpenCC for language: {}",
        effective_language
    );

    // Use OpenCC to convert based on selected language
    let config = if is_simplified {
        // Convert Traditional Chinese to Simplified Chinese
        BuiltinConfig::Tw2sp
    } else {
        // Convert Simplified Chinese to Traditional Chinese
        BuiltinConfig::S2tw
    };

    match OpenCC::from_config(config) {
        Ok(converter) => {
            let converted = converter.convert(transcription);
            debug!(
                "OpenCC translation completed. Input length: {}, Output length: {}",
                transcription.len(),
                converted.len()
            );
            Some(converted)
        }
        Err(e) => {
            error!("Failed to initialize OpenCC converter: {}. Falling back to original transcription.", e);
            None
        }
    }
}

pub(crate) struct ProcessedTranscription {
    pub final_text: String,
    pub post_processed_text: Option<String>,
    pub post_process_prompt: Option<String>,
}

/// 成功转写的唯一保存入口：附件状态只决定 file_name，不能绕过文字持久化。
fn persist_completed_transcription<T>(
    wav_saved: bool,
    file_name: String,
    transcription: String,
    post_process: bool,
    processed: &ProcessedTranscription,
    save: impl FnOnce(String, String, bool, Option<String>, Option<String>) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    save(
        if wav_saved { file_name } else { String::new() },
        transcription,
        post_process,
        processed.post_processed_text.clone(),
        processed.post_process_prompt.clone(),
    )
}

/// Resolve the persisted language *intent* into the language the currently-loaded
/// model will actually use — the same capability-aware coercion the transcription
/// paths apply (see [`crate::managers::model::effective_language`]). Post-processing
/// resolves it independently so it agrees with the language the transcription ran
/// in, without threading a value through the pipeline.
fn resolve_effective_language(app: &AppHandle, settings: &AppSettings) -> String {
    let tm = app.state::<Arc<TranscriptionManager>>();
    let model_manager = app.state::<Arc<ModelManager>>();
    let active_model = tm
        .get_current_model()
        .unwrap_or_else(|| settings.selected_model.clone());
    match model_manager.get_model_info(&active_model) {
        Some(info) => crate::managers::model::effective_language(
            &settings.selected_language,
            &info.supported_languages,
            info.supports_language_detection,
        ),
        None => settings.selected_language.clone(),
    }
}

pub(crate) async fn process_transcription_output(
    app: &AppHandle,
    transcription: &str,
    post_process: bool,
) -> ProcessedTranscription {
    let settings = get_settings(app);
    let mut final_text = transcription.to_string();
    let mut post_processed_text: Option<String> = None;
    let mut post_process_prompt: Option<String> = None;

    // Resolve the language the transcription actually ran in (the persisted
    // intent coerced against the loaded model's capabilities) so OpenCC keys off
    // the effective language rather than a possibly-stale intent.
    let effective_language = resolve_effective_language(app, &settings);
    if let Some(converted_text) =
        maybe_convert_chinese_variant(&effective_language, transcription).await
    {
        final_text = converted_text;
    }

    let local_provider_selected = post_process
        && settings
            .active_post_process_provider()
            .is_some_and(|provider| provider.id == LOCAL_POST_PROCESS_PROVIDER_ID);
    let allow_custom_word_correction = if local_provider_selected {
        false
    } else {
        local_decision_allows_custom_words(final_text.clone(), settings.custom_words.clone()).await
    };
    if allow_custom_word_correction {
        if let Some(corrected_text) =
            crate::custom_words_model::correct_custom_words(app, &settings, &final_text).await
        {
            if corrected_text != final_text {
                post_processed_text = Some(corrected_text.clone());
                final_text = corrected_text;
            }
        }
    }

    if post_process {
        if let Some(processed_text) = post_process_transcription(app, &settings, &final_text).await
        {
            post_processed_text = Some(processed_text.clone());
            final_text = processed_text;

            if let Some(prompt_id) = &settings.post_process_selected_prompt_id {
                if let Some(prompt) = settings
                    .post_process_prompts
                    .iter()
                    .find(|prompt| &prompt.id == prompt_id)
                {
                    post_process_prompt = Some(prompt.prompt.clone());
                }
            }
        }
    } else if final_text != transcription {
        post_processed_text = Some(final_text.clone());
    }

    ProcessedTranscription {
        final_text,
        post_processed_text,
        post_process_prompt,
    }
}

/// 本地判断模型只负责决定是否值得运行已有的受限自定义词纠错器。
/// 没有显式配置 worker、worker 失败或响应不可信时返回 true，保持旧行为。
async fn local_decision_allows_custom_words(text: String, custom_words: Vec<String>) -> bool {
    if crate::decision_worker::DecisionWorker::configured().is_none() {
        return true;
    }
    let request = DecisionRequest {
        protocol_version: inputia_handy_runtime::decision::PROTOCOL_VERSION,
        request_id: format!("route-{}", uuid_like_id()),
        model_id: "laya-multilingual-mlx".into(),
        state: serde_json::json!({ "text": text, "custom_words": custom_words }),
        questions: [(
            "route".into(),
            DecisionQuestion::Choice {
                instructions: "Choose the safest processing route. Choose keep unless an authorized custom term correction is clearly needed.".into(),
                criteria: [
                    ("keep".into(), "No change is needed".into()),
                    ("custom_word".into(), "Only an authorized custom-word correction is needed".into()),
                ]
                .into_iter()
                .collect(),
            },
        )]
        .into_iter()
        .collect(),
        limits: DecisionLimits {
            deadline_ms: 180,
            max_input_chars: 4_000,
        },
    };
    let response: Option<DecisionResponse> = tokio::time::timeout(
        Duration::from_millis(180),
        tauri::async_runtime::spawn_blocking(move || crate::decision_worker::request(request)),
    )
    .await
    .ok()
    .and_then(|result| result.ok())
    .flatten();
    response
        .and_then(|value| {
            value.answers.get("route").and_then(|answer| match answer {
                inputia_handy_runtime::decision::DecisionAnswer::Choice {
                    selected,
                    abstained,
                    ..
                } if !abstained => Some(selected == "custom_word"),
                _ => None,
            })
        })
        .unwrap_or(true)
}

fn uuid_like_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos().to_string())
        .unwrap_or_else(|_| "0".into())
}

impl ShortcutAction for TranscribeAction {
    fn start(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        #[cfg(target_os = "macos")]
        if crate::voice_connection::candidate_listener_stopping(app) {
            return;
        }
        let start_time = Instant::now();
        debug!("TranscribeAction::start called for binding: {}", binding_id);

        // Load model in the background
        let tm = app.state::<Arc<TranscriptionManager>>();
        let rm = app.state::<Arc<AudioRecordingManager>>();

        // Load ASR model and VAD model in parallel
        let kickoff_started = Instant::now();
        tm.initiate_model_load();
        let rm_clone = Arc::clone(&rm);
        std::thread::spawn(move || {
            if let Err(e) = rm_clone.preload_vad() {
                debug!("VAD pre-load failed: {}", e);
            }
        });
        let kickoff_elapsed = kickoff_started.elapsed();

        let binding_id = binding_id.to_string();
        let tray_started = Instant::now();
        set_tray_state(app, TrayIconState::Recording);
        let tray_elapsed = tray_started.elapsed();

        // Get the microphone mode to determine audio feedback timing
        let plan_started = Instant::now();
        let settings = get_settings(app);
        let is_always_on = settings.always_on_microphone;

        let selected_model_info = app
            .state::<Arc<ModelManager>>()
            .get_model_info(&settings.selected_model);

        // Use the app-facing model capability as the single pre-recording source
        // for live streaming decisions. Unknown support is represented as false
        // until the model registry is updated by discovery or runtime load.
        let model_supports_streaming = selected_model_info
            .as_ref()
            .map(|m| m.supports_streaming)
            .unwrap_or(false);
        let vad_policy = if !settings.vad_enabled {
            VadPolicy::Disabled
        } else if model_supports_streaming {
            VadPolicy::Streaming
        } else {
            VadPolicy::Offline
        };
        if model_supports_streaming {
            let owned_voice = app
                .try_state::<TranscriptionCoordinator>()
                .and_then(|coordinator| coordinator.voice_output_context());
            if owned_voice.is_some() {
                tm.start_stream_with_voice_context(owned_voice);
            } else {
                tm.start_stream();
            }
        }
        let plan_elapsed = plan_started.elapsed();

        // Sizing the overlay follows the same advertised capability. A model that
        // doesn't stream (or whose capability is not known yet) gets the compact
        // pill instead of an oversized transparent live window.
        let overlay_started = Instant::now();
        match settings.overlay_style {
            OverlayStyle::Live if model_supports_streaming => utils::show_streaming_overlay(app),
            OverlayStyle::Live | OverlayStyle::Minimal => show_recording_overlay(app),
            OverlayStyle::None => {} // show_overlay_state no-ops on None anyway
        }
        // Everything above runs before capture can begin, so each span here is
        // added keypress->capture latency.
        debug!(
            "start-path pre-recording steps: model_kickoff={:?} tray={:?} settings+stream_plan={:?} overlay={:?}",
            kickoff_elapsed,
            tray_elapsed,
            plan_elapsed,
            overlay_started.elapsed()
        );
        debug!("Microphone mode - always_on: {}", is_always_on);

        let mut recording_error: Option<String> = None;
        let recording_start_time = Instant::now();
        match rm.try_start_recording(&binding_id, vad_policy) {
            Ok(readiness) => {
                debug!(
                    "Recording request accepted in {:?}; waiting for first microphone samples",
                    recording_start_time.elapsed()
                );
                let generation = readiness.generation();
                if let Some(coordinator) = app.try_state::<TranscriptionCoordinator>() {
                    coordinator.notify_recording_requested(&binding_id, generation);
                }
                let binding_for_ready = binding_id.to_owned();
                let owned_voice = app
                    .try_state::<TranscriptionCoordinator>()
                    .and_then(|coordinator| coordinator.voice_output_context())
                    .is_some();
                let app_clone = app.clone();
                let rm_clone = Arc::clone(&rm);
                std::thread::spawn(move || {
                    let ready = if owned_voice {
                        readiness.wait_timeout(Duration::from_secs(15))
                    } else if readiness.wait() {
                        Ok(())
                    } else {
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
                    };
                    if let Err(error) = ready {
                        // owned首帧前断开同样是准备失败；正常Stop/Cancel及旧generation
                        // 由协调器的归属/阶段校验过滤，不能让等待线程退出后永远Preparing。
                        if owned_voice {
                            if let Some(coordinator) =
                                app_clone.try_state::<TranscriptionCoordinator>()
                            {
                                coordinator
                                    .notify_preparation_failed(&binding_for_ready, generation);
                            }
                        }
                        debug!(
                            "Microphone readiness wait ended without receiving samples: {error:?}"
                        );
                        return;
                    }

                    // Development-only preview hook for evaluating the brief
                    // arming animation on hardware that normally starts too fast
                    // to make it visible.
                    #[cfg(debug_assertions)]
                    if let Ok(delay_ms) = std::env::var("HANDY_DEBUG_MIC_READY_DELAY_MS")
                        .unwrap_or_default()
                        .parse::<u64>()
                    {
                        let delay_ms = delay_ms.min(10_000);
                        if delay_ms > 0 {
                            debug!("Delaying microphone-ready cue by {delay_ms}ms for UI preview");
                            std::thread::sleep(Duration::from_millis(delay_ms));
                        }
                    }

                    if !rm_clone.is_recording_readiness_current(generation) {
                        debug!("Microphone became ready for an inactive recording");
                        return;
                    }

                    debug!("Microphone is receiving samples; recording is ready");
                    if let Some(coordinator) = app_clone.try_state::<TranscriptionCoordinator>() {
                        coordinator.notify_recording_ready(&binding_for_ready, generation);
                    }
                    utils::emit_recording_ready(&app_clone);

                    // The start chime is a readiness cue, so it must follow the
                    // first real input callback rather than Stream::play() or a
                    // fixed delay. The helper returns immediately when feedback
                    // is disabled; mute still follows the same readiness point.
                    if rm_clone.is_recording_readiness_current(generation) {
                        play_feedback_sound_blocking(&app_clone, SoundType::Start);
                    }
                    if rm_clone.is_recording_readiness_current(generation) {
                        rm_clone.apply_mute();
                    }
                });
            }
            Err(e) => {
                debug!("Failed to start recording: {}", e);
                recording_error = Some(e);
            }
        }

        if recording_error.is_none() {
            // Dynamically register the cancel shortcut in a separate task to avoid deadlock
            shortcut::register_cancel_shortcut(app);
        } else {
            // Starting failed (for example due to blocked microphone permissions).
            // Revert UI state so we don't stay stuck in the recording overlay.
            tm.cancel_stream();
            utils::hide_recording_overlay(app);
            set_tray_state(app, TrayIconState::Idle);
            if let Some(err) = recording_error {
                let error_type = if is_microphone_access_denied(&err) {
                    "microphone_permission_denied"
                } else if is_no_input_device_error(&err) {
                    "no_input_device"
                } else {
                    "unknown"
                };
                let _ = app.emit(
                    "recording-error",
                    RecordingErrorEvent {
                        error_type: error_type.to_string(),
                        detail: Some(err),
                    },
                );
            }
        }

        debug!(
            "TranscribeAction::start completed in {:?}",
            start_time.elapsed()
        );
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, _shortcut_str: &str) {
        // Prevent a slow microphone from emitting a ready event or start chime
        // after the user has already requested stop.
        app.state::<Arc<AudioRecordingManager>>()
            .invalidate_recording_readiness();

        // Unregister the cancel shortcut when transcription stops
        shortcut::unregister_cancel_shortcut(app);

        let stop_time = Instant::now();
        debug!("TranscribeAction::stop called for binding: {}", binding_id);

        let ah = app.clone();
        let rm = Arc::clone(&app.state::<Arc<AudioRecordingManager>>());
        let tm = Arc::clone(&app.state::<Arc<TranscriptionManager>>());
        let hm = Arc::clone(&app.state::<Arc<HistoryManager>>());
        let owned_voice = app
            .try_state::<TranscriptionCoordinator>()
            .and_then(|coordinator| coordinator.voice_output_context());
        #[cfg(target_os = "macos")]
        let platform_voice = app
            .try_state::<TranscriptionCoordinator>()
            .and_then(|coordinator| coordinator.take_platform_output_context());

        set_tray_state(app, TrayIconState::Transcribing);
        // Stop should give immediate visual feedback. Live streaming can keep
        // the larger panel, but it still switches from listening to a working
        // spinner while the stream finalizes. Non-streaming paths use the
        // compact transcribing pill (None no-ops in show_*).
        let style = get_settings(app).overlay_style;
        // Capture this before finalizing the stream so every later working state
        // targets the same overlay that was shown for this transcription.
        let use_streaming_overlay = should_use_streaming_overlay(style, tm.is_streaming());
        if use_streaming_overlay {
            tm.emit_stream_working(StreamWorkKind::Transcribing);
        } else {
            show_transcribing_overlay(app);
        }

        // Unmute before playing audio feedback so the stop sound is audible
        rm.remove_mute();

        // Play audio feedback for recording stop
        play_feedback_sound(app, SoundType::Stop);

        let binding_id = binding_id.to_string(); // Clone binding_id for the async task
        let post_process = self.post_process;
        let cancel_generation = rm.cancel_generation();
        let input_permission_epoch = crate::input_permission::capture_epoch();

        tauri::async_runtime::spawn(async move {
            let _guard = FinishGuard(ah.clone());
            debug!(
                "Starting async transcription task for binding: {}",
                binding_id
            );

            let stop_recording_time = Instant::now();
            if let Some(samples) = rm.stop_recording(&binding_id, cancel_generation) {
                debug!(
                    "Recording stopped and samples retrieved in {:?}, sample count: {}",
                    stop_recording_time.elapsed(),
                    samples.len()
                );

                if rm.was_cancelled_since(cancel_generation) {
                    debug!("Transcription operation cancelled after recording stop");
                    tm.cancel_stream();
                    utils::hide_recording_overlay(&ah);
                    set_tray_state(&ah, TrayIconState::Idle);
                    return;
                }

                if samples.is_empty() {
                    debug!("Recording produced no audio samples; skipping persistence");
                    // Tear down any streaming worker so its channel doesn't leak
                    // and block the next start_stream.
                    tm.cancel_stream();
                    utils::hide_recording_overlay(&ah);
                    set_tray_state(&ah, TrayIconState::Idle);
                } else {
                    // Save WAV concurrently with transcription
                    let sample_count = samples.len();
                    let file_name = format!("handy-{}.wav", chrono::Utc::now().timestamp());
                    let wav_path = hm.recordings_dir().join(&file_name);
                    let wav_path_for_verify = wav_path.clone();
                    let samples_for_wav = samples.clone();
                    let wav_handle = tauri::async_runtime::spawn_blocking(move || {
                        crate::audio_toolkit::save_wav_file(&wav_path, &samples_for_wav)
                    });

                    // Transcribe concurrently with WAV save. If a live stream was
                    // running, finalize it and use its text (all audio was already
                    // fed to the stream); otherwise batch-transcribe the samples.
                    let transcription_time = Instant::now();
                    let transcription_result = match tm.finalize_stream() {
                        // A finalized stream with usable text wins. An empty result
                        // (no active stream, produced nothing, or a finalize error
                        // after the engine was returned) falls back to a full batch
                        // transcription of the same audio. A finalize timeout is
                        // surfaced instead — the worker may still hold the engine,
                        // so a batch fallback would contend with it.
                        Ok(Some(text)) if !text.trim().is_empty() => Ok(text),
                        Ok(_) => tm.transcribe_with_voice_context(samples, owned_voice.as_ref()),
                        Err(err) => Err(err),
                    };

                    // Await WAV save and verify
                    let wav_saved = match wav_handle.await {
                        Ok(Ok(())) => {
                            match crate::audio_toolkit::verify_wav_file(
                                &wav_path_for_verify,
                                sample_count,
                            ) {
                                Ok(()) => true,
                                Err(e) => {
                                    error!("WAV verification failed: {}", e);
                                    false
                                }
                            }
                        }
                        Ok(Err(e)) => {
                            error!("Failed to save WAV file: {}", e);
                            false
                        }
                        Err(e) => {
                            error!("WAV save task panicked: {}", e);
                            false
                        }
                    };

                    if rm.was_cancelled_since(cancel_generation) {
                        debug!("Transcription operation cancelled before output handling");
                        utils::hide_recording_overlay(&ah);
                        set_tray_state(&ah, TrayIconState::Idle);
                        return;
                    }

                    match transcription_result {
                        Ok(transcription) => {
                            debug!(
                                "Transcription completed in {:?}: '{}'",
                                transcription_time.elapsed(),
                                utils::redact_text(&transcription)
                            );

                            if post_process {
                                if use_streaming_overlay {
                                    tm.emit_stream_working(StreamWorkKind::Polishing);
                                } else {
                                    show_processing_overlay(&ah);
                                }
                            }
                            let Some(processed) = complete_unless_cancelled(
                                process_transcription_output(&ah, &transcription, post_process),
                                || rm.was_cancelled_since(cancel_generation),
                            )
                            .await
                            else {
                                debug!("Transcription operation cancelled during output handling");
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                                return;
                            };

                            if rm.was_cancelled_since(cancel_generation) {
                                debug!("Transcription operation cancelled before paste");
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                                return;
                            }

                            // 文字是主体，音频只是可选附件；音频写盘/验证失败不能丢掉成功文字。
                            let voice_source = owned_voice.as_ref().and_then(
                                crate::managers::history::VerifiedVoiceSource::from_owned_request,
                            );
                            let saved_entry = persist_completed_transcription(
                                wav_saved,
                                file_name,
                                transcription,
                                post_process,
                                &processed,
                                |file, text, requested, processed_text, prompt| {
                                    hm.save_entry_with_voice_source(
                                        file,
                                        text,
                                        requested,
                                        processed_text,
                                        prompt,
                                        voice_source.as_ref(),
                                    )
                                },
                            );
                            if let Err(err) = &saved_entry {
                                error!("Failed to save history entry: {}", err);
                            }

                            // macOS Legacy 在账本中留下待插入结果，即使录音期间权限变化。
                            if owned_voice.is_some() || cfg!(not(target_os = "macos")) {
                                if let Err(error) = input_permission_epoch
                                    .as_ref()
                                    .map_err(Clone::clone)
                                    .and_then(|epoch| crate::input_permission::check_epoch(*epoch))
                                {
                                    debug!("Discarding stale transcription output: {error}");
                                    utils::hide_recording_overlay(&ah);
                                    set_tray_state(&ah, TrayIconState::Idle);
                                    return;
                                }
                            }
                            if let Some(start) = owned_voice {
                                // 归属在Stop时冻结；准备失败也不能落入平台paste成为第二所有者。
                                if !processed.final_text.is_empty() {
                                    let result = match saved_entry {
                                        Ok(entry) => {
                                            prepare_owned_voice_result(
                                                &ah,
                                                start,
                                                entry.id,
                                                processed.final_text,
                                                *input_permission_epoch
                                                    .as_ref()
                                                    .map_err(Clone::clone)
                                                    .unwrap_or(&0),
                                            )
                                            .await
                                        }
                                        Err(_) => Err("语音历史保存失败，未派发输入法输出".into()),
                                    };
                                    if let Err(error) = result {
                                        error!("Owned voice result preparation failed: {error}");
                                        let _ = ah.emit("paste-error", ());
                                    }
                                }
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                                return;
                            }

                            #[cfg(target_os = "macos")]
                            {
                                if !processed.final_text.is_empty() {
                                    let result = match saved_entry {
                                        Ok(entry) => {
                                            let output_app = ah.clone();
                                            let cancelled_audio = Arc::clone(&rm);
                                            let cancelled = Arc::new(move || {
                                                cancelled_audio
                                                    .was_cancelled_since(cancel_generation)
                                            });
                                            tauri::async_runtime::spawn_blocking(move || {
                                                crate::integration_output::dispatch_saved_platform_result(
                                                    output_app, platform_voice, entry.id, processed.final_text, cancelled,
                                                )
                                            }).await.map_err(|_| "platform output worker interrupted".to_owned())
                                                .and_then(|result| result)
                                        }
                                        Err(_) => Err("语音历史保存失败，未派发平台输出".into()),
                                    };
                                    match result {
                                        Ok(output) => {
                                            use inputia_handy_runtime::output_ledger::OutputState;
                                            let state = match output.state {
                                                OutputState::Confirmed => "confirmed",
                                                OutputState::DispatchedOnly => "dispatched",
                                                OutputState::PendingTarget
                                                | OutputState::Prepared => "pending_target",
                                                OutputState::Dispatched
                                                | OutputState::Uncertain => "uncertain",
                                                OutputState::Rejected => "rejected",
                                            };
                                            let _ = ah.emit(
                                                "voice-output-result",
                                                serde_json::json!({
                                                    "operation_id": output.intent.operation_id,
                                                    "item_id": output.intent.item_id,
                                                    "state": state,
                                                }),
                                            );
                                        }
                                        Err(error) => {
                                            error!("Platform voice output failed: {error}");
                                            let _ = ah.emit("paste-error", ());
                                        }
                                    }
                                }
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                            }
                            #[cfg(not(target_os = "macos"))]
                            {
                                if processed.final_text.is_empty() {
                                    utils::hide_recording_overlay(&ah);
                                    set_tray_state(&ah, TrayIconState::Idle);
                                } else {
                                    let ah_clone = ah.clone();
                                    let paste_time = Instant::now();
                                    let final_text = processed.final_text;
                                    let rm_for_paste = Arc::clone(&rm);
                                    let permission_epoch = input_permission_epoch;
                                    ah.run_on_main_thread(move || {
                                        let _permission_scope = match permission_epoch
                                            .and_then(crate::input_permission::RequestScope::enter)
                                        {
                                            Ok(scope) => scope,
                                            Err(error) => {
                                                debug!("Discarding stale queued paste: {error}");
                                                utils::hide_recording_overlay(&ah_clone);
                                                set_tray_state(&ah_clone, TrayIconState::Idle);
                                                return;
                                            }
                                        };
                                        if rm_for_paste.was_cancelled_since(cancel_generation) {
                                            debug!(
                                                "Transcription operation cancelled before paste"
                                            );
                                            utils::hide_recording_overlay(&ah_clone);
                                            set_tray_state(&ah_clone, TrayIconState::Idle);
                                            return;
                                        }

                                        match utils::paste(final_text, ah_clone.clone()) {
                                            Ok(()) => debug!(
                                                "Text pasted successfully in {:?}",
                                                paste_time.elapsed()
                                            ),
                                            Err(e) => {
                                                error!("Failed to paste transcription: {}", e);
                                                let _ = ah_clone.emit("paste-error", ());
                                            }
                                        }
                                        utils::hide_recording_overlay(&ah_clone);
                                        set_tray_state(&ah_clone, TrayIconState::Idle);
                                    })
                                    .unwrap_or_else(|e| {
                                        error!("Failed to run paste on main thread: {:?}", e);
                                        utils::hide_recording_overlay(&ah);
                                        set_tray_state(&ah, TrayIconState::Idle);
                                    });
                                }
                            }
                        }
                        Err(err) => {
                            if rm.was_cancelled_since(cancel_generation) {
                                debug!(
                                    "Transcription operation cancelled after transcription error"
                                );
                                utils::hide_recording_overlay(&ah);
                                set_tray_state(&ah, TrayIconState::Idle);
                                return;
                            }

                            error!("Transcription failed: {}", err);
                            // Surface the failure to the UI (toast). The full
                            // message is also in handy.log via the line above.
                            let _ = ah.emit("transcription-error", err.to_string());
                            // Save entry with empty text so user can retry
                            if wav_saved {
                                if let Err(save_err) = hm.save_entry(
                                    file_name,
                                    String::new(),
                                    post_process,
                                    None,
                                    None,
                                ) {
                                    error!("Failed to save failed history entry: {}", save_err);
                                }
                            }
                            utils::hide_recording_overlay(&ah);
                            set_tray_state(&ah, TrayIconState::Idle);
                        }
                    }
                }
            } else {
                debug!("No samples retrieved from recording stop");
                // Tear down any streaming worker so its channel doesn't leak.
                tm.cancel_stream();
                utils::hide_recording_overlay(&ah);
                set_tray_state(&ah, TrayIconState::Idle);
            }
        });

        debug!(
            "TranscribeAction::stop completed in {:?}",
            stop_time.elapsed()
        );
    }
}

// Cancel Action
struct CancelAction;

impl ShortcutAction for CancelAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        utils::cancel_current_operation(app);
    }

    fn stop(&self, _app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        // Nothing to do on stop for cancel
    }
}

struct ClipboardHistoryAction;

impl ShortcutAction for ClipboardHistoryAction {
    fn start(&self, app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {
        if crate::settings::get_settings(app).clipboard_hotkey_enabled {
            crate::overlay::toggle_clipboard_overlay(app);
        }
    }

    fn stop(&self, _app: &AppHandle, _binding_id: &str, _shortcut_str: &str) {}
}

// Test Action
struct TestAction;

impl ShortcutAction for TestAction {
    fn start(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        log::info!(
            "Shortcut ID '{}': Started - {} (App: {})", // Changed "Pressed" to "Started" for consistency
            binding_id,
            shortcut_str,
            app.package_info().name
        );
    }

    fn stop(&self, app: &AppHandle, binding_id: &str, shortcut_str: &str) {
        log::info!(
            "Shortcut ID '{}': Stopped - {} (App: {})", // Changed "Released" to "Stopped" for consistency
            binding_id,
            shortcut_str,
            app.package_info().name
        );
    }
}

// Static Action Map
pub static ACTION_MAP: Lazy<HashMap<String, Arc<dyn ShortcutAction>>> = Lazy::new(|| {
    let mut map = HashMap::new();
    map.insert(
        "transcribe".to_string(),
        Arc::new(TranscribeAction {
            post_process: false,
        }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "transcribe_with_post_process".to_string(),
        Arc::new(TranscribeAction { post_process: true }) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "cancel".to_string(),
        Arc::new(CancelAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        crate::settings::CLIPBOARD_HISTORY_BINDING_ID.to_string(),
        Arc::new(ClipboardHistoryAction) as Arc<dyn ShortcutAction>,
    );
    map.insert(
        "test".to_string(),
        Arc::new(TestAction) as Arc<dyn ShortcutAction>,
    );
    map
});

#[cfg(test)]
mod tests {
    use super::{
        complete_unless_cancelled, is_blank_transcription, render_prompt_template,
        should_use_streaming_overlay, strip_think_block, ACTION_MAP,
    };
    use crate::settings::OverlayStyle;
    use std::future;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn successful_transcription_persists_to_real_sqlite_when_wav_write_or_verification_fails() {
        use crate::managers::history::{insert_entry_with_conn, HistoryEntry};
        for scenario in ["write-failure", "verification-failure", "verified"] {
            let temp = tempfile::tempdir().unwrap();
            let database = temp.path().join("history.db");
            let wav = temp.path().join("synthetic.wav");
            let samples = vec![0.1, -0.2, 0.3];
            let wav_saved = if scenario == "write-failure" {
                std::fs::create_dir(&wav).unwrap();
                assert!(crate::audio_toolkit::save_wav_file(&wav, &samples).is_err());
                false
            } else {
                crate::audio_toolkit::save_wav_file(&wav, &samples).unwrap();
                let expected = if scenario == "verification-failure" {
                    samples.len() + 1
                } else {
                    samples.len()
                };
                let verification = crate::audio_toolkit::verify_wav_file(&wav, expected);
                assert_eq!(verification.is_ok(), scenario == "verified");
                verification.is_ok()
            };
            let conn = rusqlite::Connection::open(&database).unwrap();
            conn.execute_batch("CREATE TABLE transcription_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT, file_name TEXT NOT NULL, timestamp INTEGER NOT NULL,
                saved BOOLEAN NOT NULL, title TEXT NOT NULL, transcription_text TEXT NOT NULL,
                post_processed_text TEXT, post_process_prompt TEXT, post_process_requested BOOLEAN NOT NULL);").unwrap();
            let processed = super::ProcessedTranscription {
                final_text: "成功文字".into(),
                post_processed_text: Some("成功文字".into()),
                post_process_prompt: Some("合成提示".into()),
            };
            let mut saves = 0;
            let saved = super::persist_completed_transcription(
                wav_saved,
                "synthetic.wav".into(),
                "原始文字".into(),
                true,
                &processed,
                |file_name,
                 transcription_text,
                 post_process_requested,
                 post_processed_text,
                 post_process_prompt| {
                    saves += 1;
                    insert_entry_with_conn(
                        &conn,
                        HistoryEntry {
                            id: 0,
                            file_name,
                            timestamp: 10,
                            saved: false,
                            title: "合成历史".into(),
                            transcription_text,
                            post_process_requested,
                            post_processed_text,
                            post_process_prompt,
                        },
                    )
                },
            )
            .unwrap();
            assert_eq!(saves, 1, "{scenario} 不能跳过成功文字保存");
            assert_eq!(
                saved.file_name,
                if wav_saved { "synthetic.wav" } else { "" }
            );
            drop(conn);
            let reopened = rusqlite::Connection::open(database).unwrap();
            let row = reopened.query_row("SELECT file_name,transcription_text,post_processed_text,post_process_prompt,post_process_requested FROM transcription_history",
                [], |row| Ok((row.get::<_,String>(0)?, row.get::<_,String>(1)?, row.get::<_,String>(2)?, row.get::<_,String>(3)?, row.get::<_,bool>(4)?))).unwrap();
            assert_eq!(
                row,
                (
                    saved.file_name,
                    "原始文字".into(),
                    "成功文字".into(),
                    "合成提示".into(),
                    true
                )
            );
        }
    }

    #[test]
    fn clipboard_history_binding_has_an_action() {
        assert!(ACTION_MAP.contains_key(crate::settings::CLIPBOARD_HISTORY_BINDING_ID));
    }

    #[test]
    fn custom_words_placeholder_is_rendered_as_json() {
        let rendered = render_prompt_template(
            "words=${custom_words}\ntext=${output}",
            "hello",
            &[" Inputia ".to_string(), "罗泽群".to_string()],
        );

        assert_eq!(rendered, "words=[\"Inputia\",\"罗泽群\"]\ntext=hello");
    }

    #[test]
    fn blank_transcription_is_detected() {
        assert!(is_blank_transcription(""));
        assert!(is_blank_transcription("   "));
        assert!(is_blank_transcription("\t\n  \r\n"));
    }

    #[test]
    fn non_blank_transcription_is_kept() {
        assert!(!is_blank_transcription("hello"));
        assert!(!is_blank_transcription("  hello  "));
    }

    #[test]
    fn completed_operation_returns_its_output() {
        let result = tauri::async_runtime::block_on(complete_unless_cancelled(
            future::ready("done"),
            || false,
        ));

        assert_eq!(result, Some("done"));
    }

    #[test]
    fn pending_operation_stops_after_cancellation() {
        let cancelled = Arc::new(AtomicBool::new(false));
        let cancelled_for_thread = Arc::clone(&cancelled);
        let cancel_thread = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            cancelled_for_thread.store(true, Ordering::Release);
        });

        let result = tauri::async_runtime::block_on(complete_unless_cancelled(
            future::pending::<()>(),
            || cancelled.load(Ordering::Acquire),
        ));

        cancel_thread.join().unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn leading_think_block_is_stripped() {
        assert_eq!(
            strip_think_block("<think>pondering...</think>Cleaned text."),
            "Cleaned text."
        );
        assert_eq!(
            strip_think_block("  \n<think>multi\nline</think>\n  Cleaned text."),
            "Cleaned text."
        );
    }

    #[test]
    fn content_without_think_block_is_unchanged() {
        assert_eq!(strip_think_block("Cleaned text."), "Cleaned text.");
        assert_eq!(
            strip_think_block("Mentions <think> mid-sentence."),
            "Mentions <think> mid-sentence."
        );
        // Unclosed block: leave untouched rather than guess
        assert_eq!(
            strip_think_block("<think>never closed"),
            "<think>never closed"
        );
    }

    #[test]
    fn live_overlay_uses_streaming_states_only_for_streaming_models() {
        assert!(should_use_streaming_overlay(OverlayStyle::Live, true));
        assert!(!should_use_streaming_overlay(OverlayStyle::Live, false));
        assert!(!should_use_streaming_overlay(OverlayStyle::Minimal, true));
        assert!(!should_use_streaming_overlay(OverlayStyle::None, true));
    }
}
