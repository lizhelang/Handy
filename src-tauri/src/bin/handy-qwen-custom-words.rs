use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use once_cell::sync::{Lazy, OnceCell};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const QWEN_CONTEXT_TOKENS: u32 = 2048;
const QWEN_BATCH_TOKENS: u32 = 512;
const QWEN_GPU_LAYERS: u32 = 99;
const QWEN_MAX_GENERATED_TOKENS: usize = 192;
const JSON_PREFILL: &str = "{\"replacements\":";
const MAX_REPLACEMENTS: usize = 16;

static QWEN_BACKEND: OnceCell<&'static LlamaBackend> = OnceCell::new();
static QWEN_MODEL_CACHE: Lazy<Mutex<Option<CachedQwenModel>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Deserialize)]
struct HelperRequest {
    model_path: PathBuf,
    system: String,
    user: String,
    source_text: String,
    custom_words: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HelperResponse {
    output: Option<String>,
    error: Option<String>,
}

#[derive(Debug)]
struct CachedQwenModel {
    path: PathBuf,
    model: LlamaModel,
}

#[derive(Debug, Deserialize)]
struct ModelReplacementResponse {
    replacements: Vec<ModelReplacement>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct ModelReplacement {
    from: String,
    to: String,
}

#[derive(Debug, Serialize)]
struct FinalCorrectionResponse {
    text: String,
    replacements: Vec<ModelReplacement>,
}

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let response = match line {
            Ok(line) => handle_line(&line),
            Err(err) => HelperResponse {
                output: None,
                error: Some(format!("failed to read helper request: {err}")),
            },
        };

        if serde_json::to_writer(&mut stdout, &response).is_err() {
            break;
        }
        if stdout
            .write_all(b"\n")
            .and_then(|_| stdout.flush())
            .is_err()
        {
            break;
        }
    }

    exit_without_destructors();
}

fn handle_line(line: &str) -> HelperResponse {
    match serde_json::from_str::<HelperRequest>(line)
        .map_err(|err| format!("invalid helper request JSON: {err}"))
        .and_then(|request| generate(&request))
    {
        Ok(output) => HelperResponse {
            output: Some(output),
            error: None,
        },
        Err(error) => HelperResponse {
            output: None,
            error: Some(error),
        },
    }
}

fn generate(request: &HelperRequest) -> Result<String, String> {
    let raw = with_cached_qwen_model(&request.model_path, |backend, model| {
        run_qwen_generation(
            backend,
            model,
            &CustomWordsPrompt {
                system: request.system.clone(),
                user: request.user.clone(),
            },
        )
    })?;

    build_final_response(&request.source_text, &request.custom_words, &raw)
}

#[derive(Debug)]
struct CustomWordsPrompt {
    system: String,
    user: String,
}

fn build_final_response(
    source_text: &str,
    custom_words: &[String],
    raw: &str,
) -> Result<String, String> {
    let replacements = parse_model_replacements(raw)?;
    let allowed_words = custom_words
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut rebuilt = source_text.to_string();
    let mut accepted = Vec::new();

    for replacement in replacements {
        if accepted.len() >= MAX_REPLACEMENTS {
            break;
        }
        if replacement.from.is_empty()
            || replacement.from == replacement.to
            || !allowed_words.contains(replacement.to.as_str())
            || !rebuilt.contains(&replacement.from)
        {
            continue;
        }

        rebuilt = rebuilt.replace(&replacement.from, &replacement.to);
        accepted.push(replacement);
    }

    serde_json::to_string(&FinalCorrectionResponse {
        text: rebuilt,
        replacements: accepted,
    })
    .map_err(|err| format!("failed to encode final correction response: {err}"))
}

fn parse_model_replacements(raw: &str) -> Result<Vec<ModelReplacement>, String> {
    match serde_json::from_str::<ModelReplacementResponse>(raw.trim()) {
        Ok(payload) => Ok(payload.replacements),
        Err(strict_err) => {
            let recovered = recover_replacements_from_partial_json(raw);
            if recovered.is_empty() {
                Err(format!(
                    "model response is not replacements JSON: {strict_err}; raw={raw:?}"
                ))
            } else {
                Ok(recovered)
            }
        }
    }
}

fn recover_replacements_from_partial_json(raw: &str) -> Vec<ModelReplacement> {
    let Some(array_start) = raw.find('[') else {
        return Vec::new();
    };

    let mut replacements = Vec::new();
    let mut object_start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for (relative_idx, ch) in raw[array_start..].char_indices() {
        let idx = array_start + relative_idx;
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    object_start = Some(idx);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some(start) = object_start.take() {
                        let end = idx + ch.len_utf8();
                        if let Ok(replacement) =
                            serde_json::from_str::<ModelReplacement>(&raw[start..end])
                        {
                            replacements.push(replacement);
                        }
                    }
                }
            }
            ']' if depth == 0 => break,
            _ => {}
        }
    }

    replacements
}

fn qwen_backend() -> Result<&'static LlamaBackend, String> {
    QWEN_BACKEND
        .get_or_try_init(|| {
            let mut backend = LlamaBackend::init()
                .map_err(|err| format!("failed to initialize llama.cpp backend: {err}"))?;
            backend.void_logs();
            Ok(Box::leak(Box::new(backend)) as &'static LlamaBackend)
        })
        .copied()
}

fn with_cached_qwen_model<T>(
    model_path: &Path,
    run: impl FnOnce(&LlamaBackend, &LlamaModel) -> Result<T, String>,
) -> Result<T, String> {
    let backend = qwen_backend()?;
    let mut cache = QWEN_MODEL_CACHE
        .lock()
        .map_err(|_| "custom-word model cache is poisoned".to_string())?;

    let should_reload = cache
        .as_ref()
        .is_none_or(|cached| cached.path.as_path() != model_path);
    if should_reload {
        let model_params = LlamaModelParams::default().with_n_gpu_layers(QWEN_GPU_LAYERS);
        let model = LlamaModel::load_from_file(backend, model_path, &model_params)
            .map_err(|err| format!("failed to load Qwen custom-word model: {err}"))?;
        *cache = Some(CachedQwenModel {
            path: model_path.to_path_buf(),
            model,
        });
    }

    let cached = cache
        .as_ref()
        .ok_or_else(|| "custom-word model cache was not initialized".to_string())?;
    run(backend, &cached.model)
}

fn run_qwen_generation(
    backend: &LlamaBackend,
    model: &LlamaModel,
    prompt: &CustomWordsPrompt,
) -> Result<String, String> {
    let rendered_prompt = format!("{}{}", render_qwen_prompt(model, prompt)?, JSON_PREFILL);
    let prompt_tokens = model
        .str_to_token(&rendered_prompt, AddBos::Always)
        .map_err(|err| format!("failed to tokenize custom-word prompt: {err}"))?;

    if prompt_tokens.is_empty() {
        return Err("custom-word prompt produced no tokens".to_string());
    }

    let context_tokens = usize::try_from(QWEN_CONTEXT_TOKENS).unwrap_or(2048);
    if prompt_tokens.len() + QWEN_MAX_GENERATED_TOKENS + 1 > context_tokens {
        return Err(format!(
            "custom-word prompt is too large: {} prompt tokens for {} context tokens",
            prompt_tokens.len(),
            QWEN_CONTEXT_TOKENS
        ));
    }

    let threads = std::thread::available_parallelism()
        .map(|count| count.get().min(8) as i32)
        .unwrap_or(4);
    let prompt_batch_tokens = u32::try_from(prompt_tokens.len()).unwrap_or(QWEN_CONTEXT_TOKENS);
    let batch_tokens = QWEN_BATCH_TOKENS.max(prompt_batch_tokens);
    let ubatch_tokens = QWEN_BATCH_TOKENS.min(batch_tokens);
    let context_params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(QWEN_CONTEXT_TOKENS))
        .with_n_batch(batch_tokens)
        .with_n_ubatch(ubatch_tokens)
        .with_n_threads(threads)
        .with_n_threads_batch(threads);
    let mut context = model
        .new_context(backend, context_params)
        .map_err(|err| format!("failed to create Qwen custom-word context: {err}"))?;

    let mut batch = LlamaBatch::new(prompt_tokens.len(), 1);
    batch
        .add_sequence(&prompt_tokens, 0, false)
        .map_err(|err| format!("failed to add prompt tokens: {err}"))?;
    context
        .decode(&mut batch)
        .map_err(|err| format!("failed to decode custom-word prompt: {err}"))?;

    let mut sampler = LlamaSampler::greedy();
    let mut decoder = encoding_rs::UTF_8.new_decoder();
    let mut output = JSON_PREFILL.to_string();
    let mut position = i32::try_from(prompt_tokens.len())
        .map_err(|_| "custom-word prompt token count overflowed i32".to_string())?;

    for _ in 0..QWEN_MAX_GENERATED_TOKENS {
        let token = sampler.sample(&context, -1);
        sampler.accept(token);

        if model.is_eog_token(token) {
            break;
        }

        let piece = model
            .token_to_piece(token, &mut decoder, false, None)
            .map_err(|err| format!("failed to decode Qwen output token: {err}"))?;
        output.push_str(&piece);

        if has_complete_json_object(&output) {
            break;
        }

        let mut next = LlamaBatch::new(1, 1);
        next.add(token, position, &[0], true)
            .map_err(|err| format!("failed to add generated token: {err}"))?;
        context
            .decode(&mut next)
            .map_err(|err| format!("failed to decode generated token: {err}"))?;
        position += 1;
    }

    let mut tail = String::new();
    let _ = decoder.decode_to_string(b"", &mut tail, true);
    output.push_str(&tail);
    Ok(output.trim().to_string())
}

fn render_qwen_prompt(model: &LlamaModel, prompt: &CustomWordsPrompt) -> Result<String, String> {
    let chat = [
        LlamaChatMessage::new("system".to_string(), prompt.system.clone())
            .map_err(|err| format!("failed to build system message: {err}"))?,
        LlamaChatMessage::new("user".to_string(), prompt.user.clone())
            .map_err(|err| format!("failed to build user message: {err}"))?,
    ];

    let template = match model.chat_template(None) {
        Ok(template) => template,
        Err(_) => LlamaChatTemplate::new("chatml")
            .map_err(|err| format!("failed to build ChatML template: {err}"))?,
    };

    model
        .apply_chat_template(&template, &chat, true)
        .or_else(|_| render_chatml_prompt(prompt))
}

fn render_chatml_prompt(prompt: &CustomWordsPrompt) -> Result<String, String> {
    if prompt.system.contains('\0') || prompt.user.contains('\0') {
        return Err("custom-word prompt contains null byte".to_string());
    }

    Ok(format!(
        "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
        prompt.system, prompt.user
    ))
}

fn has_complete_json_object(text: &str) -> bool {
    let mut started = false;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    for ch in text.trim_start().chars() {
        if !started {
            if ch != '{' {
                return false;
            }
            started = true;
            depth = 1;
            continue;
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return true;
                }
            }
            _ => {}
        }
    }

    false
}

#[cfg(unix)]
fn exit_without_destructors() -> ! {
    unsafe extern "C" {
        fn _exit(status: i32) -> !;
    }

    unsafe { _exit(0) }
}

#[cfg(not(unix))]
fn exit_without_destructors() -> ! {
    std::process::exit(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_response_recovers_valid_replacement_from_partial_json() {
        let raw = r#"{"replacements":[{"from":"欧拉玛","to":"ollama"},{"from":""#;
        let output =
            build_final_response("我希望先接一下欧拉玛吧。", &["ollama".to_string()], raw).unwrap();

        assert_eq!(
            output,
            r#"{"text":"我希望先接一下ollama吧。","replacements":[{"from":"欧拉玛","to":"ollama"}]}"#
        );
    }

    #[test]
    fn final_response_replaces_every_occurrence_of_valid_span() {
        let raw = r#"{"replacements":[{"from":"欧拉玛","to":"ollama"}]}"#;
        let output =
            build_final_response("欧拉玛和欧拉玛都要改。", &["ollama".to_string()], raw).unwrap();

        assert_eq!(
            output,
            r#"{"text":"ollama和ollama都要改。","replacements":[{"from":"欧拉玛","to":"ollama"}]}"#
        );
    }

    #[test]
    fn final_response_skips_replacements_that_are_not_safe() {
        let raw = r#"{"replacements":[{"from":"欧拉玛","to":"ollama"},{"from":"openai","to":"OpenAI"},{"from":"ollama","to":"ollama"}]}"#;
        let output = build_final_response(
            "我希望先接一下欧拉玛吧。",
            &["ollama".to_string(), "OpenAI".to_string()],
            raw,
        )
        .unwrap();

        assert_eq!(
            output,
            r#"{"text":"我希望先接一下ollama吧。","replacements":[{"from":"欧拉玛","to":"ollama"}]}"#
        );
    }
}
