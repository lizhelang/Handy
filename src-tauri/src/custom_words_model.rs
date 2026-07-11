use crate::settings::AppSettings;
use log::debug;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use tauri::AppHandle;

pub(crate) const CUSTOM_WORDS_MODEL_ID: &str = "qwen3-0.6b-custom-words";
const CUSTOM_WORDS_MODEL_DIR: &str = "custom-words-qwen3-0.6b";
const QWEN_HELPER_BIN: &str = "handy-qwen-custom-words";
const MAX_REPLACEMENTS: usize = 16;

static QWEN_HELPER: Lazy<Mutex<Option<QwenHelperProcess>>> = Lazy::new(|| Mutex::new(None));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CustomWordsModelStatus {
    Installed,
    Missing,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Type)]
pub struct CustomWordsModelInfo {
    pub id: String,
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub is_selected: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct CustomWordsModelResource {
    pub id: String,
    pub directory: PathBuf,
    pub model_path: Option<PathBuf>,
    pub status: CustomWordsModelStatus,
}

impl CustomWordsModelResource {
    pub(crate) fn resolve(app: &AppHandle, settings: &AppSettings) -> Self {
        match crate::portable::app_data_dir(app) {
            Ok(app_data_dir) => Self::from_models_root_with_selection(
                app_data_dir.join("models"),
                &settings.selected_custom_words_model,
            ),
            Err(err) => {
                debug!("Custom-word model resource unavailable: {}", err);
                Self {
                    id: CUSTOM_WORDS_MODEL_ID.to_string(),
                    directory: PathBuf::new(),
                    model_path: None,
                    status: CustomWordsModelStatus::Unavailable,
                }
            }
        }
    }

    pub(crate) fn from_models_root_with_selection(
        models_root: impl AsRef<Path>,
        selected_model_id: &str,
    ) -> Self {
        let directory = models_root.as_ref().join(CUSTOM_WORDS_MODEL_DIR);
        let selected_model_id = selected_model_id.trim();
        let model_path = find_model_file(&directory, selected_model_id);
        let status = if model_path.is_some() {
            CustomWordsModelStatus::Installed
        } else {
            CustomWordsModelStatus::Missing
        };

        Self {
            id: if selected_model_id.is_empty() {
                CUSTOM_WORDS_MODEL_ID.to_string()
            } else {
                selected_model_id.to_string()
            },
            directory,
            model_path,
            status,
        }
    }
}

#[derive(Debug)]
struct CustomWordsPrompt {
    system: String,
    user: String,
    source_text: String,
    custom_words: Vec<String>,
}

#[derive(Debug, Serialize)]
struct HelperRequest {
    model_path: PathBuf,
    system: String,
    user: String,
    source_text: String,
    custom_words: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct HelperResponse {
    output: Option<String>,
    error: Option<String>,
}

trait CustomWordsCorrectionRuntime {
    fn generate(&self, prompt: &CustomWordsPrompt) -> Result<Option<String>, String>;
}

#[derive(Debug)]
struct SelfManagedQwenRuntime {
    resource: CustomWordsModelResource,
}

#[derive(Debug)]
struct QwenHelperProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl SelfManagedQwenRuntime {
    fn new(resource: CustomWordsModelResource) -> Self {
        Self { resource }
    }
}

impl CustomWordsCorrectionRuntime for SelfManagedQwenRuntime {
    fn generate(&self, prompt: &CustomWordsPrompt) -> Result<Option<String>, String> {
        if self.resource.status != CustomWordsModelStatus::Installed {
            return Err(format!(
                "{} resource is not installed in {}",
                self.resource.id,
                self.resource.directory.display()
            ));
        }

        let model_path = self
            .resource
            .model_path
            .as_ref()
            .ok_or_else(|| "installed custom-word model has no model path".to_string())?;

        run_qwen_helper(model_path, prompt)
    }
}

#[derive(Debug, Deserialize)]
struct CorrectionResponse {
    text: String,
    replacements: Vec<Replacement>,
}

#[derive(Debug, Deserialize)]
struct Replacement {
    from: String,
    to: String,
}

pub(crate) async fn correct_custom_words(
    app: &AppHandle,
    settings: &AppSettings,
    transcription: &str,
) -> Option<String> {
    let resource = CustomWordsModelResource::resolve(app, settings);
    let settings = settings.clone();
    let transcription = transcription.to_string();

    match tauri::async_runtime::spawn_blocking(move || {
        let runtime = SelfManagedQwenRuntime::new(resource);
        correct_custom_words_with_runtime(&runtime, &settings, &transcription)
    })
    .await
    {
        Ok(result) => result,
        Err(err) => {
            debug!("Custom-word correction worker failed: {}", err);
            None
        }
    }
}

fn correct_custom_words_with_runtime(
    runtime: &dyn CustomWordsCorrectionRuntime,
    settings: &AppSettings,
    transcription: &str,
) -> Option<String> {
    if transcription.trim().is_empty() {
        return None;
    }

    let custom_words = eligible_custom_words(&settings.custom_words);
    if custom_words.is_empty() {
        return None;
    }

    let prompt = build_prompt(transcription, &custom_words);
    debug!(
        "Starting self-managed custom-word correction prompt (system={} chars, user={} chars)",
        prompt.system.len(),
        prompt.user.len()
    );

    let raw = match runtime.generate(&prompt) {
        Ok(Some(raw)) => raw,
        Ok(None) => return None,
        Err(err) => {
            debug!("Custom-word model correction skipped: {}", err);
            return None;
        }
    };

    match validate_model_response(transcription, &custom_words, &raw) {
        Ok(Some(corrected)) => Some(corrected),
        Ok(None) => None,
        Err(err) => {
            debug!("Rejected custom-word model output: {}", err);
            None
        }
    }
}

fn build_prompt(transcription: &str, custom_words: &[String]) -> CustomWordsPrompt {
    let system = [
        "You are Handy's local custom-word corrector for speech transcription.",
        "Only replace mistaken spans in text with exact entries from custom_words.",
        "Do not polish, reorder, add, delete, or alter any unrelated text.",
        "Do not output reasoning, markdown, code fences, or explanations.",
        "Correct near-homophone mistakes across Latin spelling and CJK transliteration when the source span clearly names a custom word.",
        "For each replacement, from must be copied from source_text before any replacement.",
        "The assistant response already starts with {\"replacements\":; continue that JSON object only.",
        "Return only strict JSON: {\"replacements\":[{\"from\":\"...\",\"to\":\"...\"}]}",
    ]
    .join("\n");

    let user = json!({
        "source_text": transcription,
        "custom_words": custom_words,
        "rules": [
            "Each replacements[].from must be a substring copied from source_text.",
            "Each replacements[].to must exactly equal one custom_words entry.",
            "Do not use a custom_words entry as replacements[].from unless it already appears in source_text.",
            "If no safe correction exists, return original text and an empty replacements array."
        ],
        "examples": [
            {
                "custom_words": ["罗泽群"],
                "text": "罗德群今天回家吗?",
                "output": {
                    "replacements": [{"from": "罗德群", "to": "罗泽群"}]
                }
            },
            {
                "custom_words": ["武清"],
                "text": "武青在天津的西北边。",
                "output": {
                    "replacements": [{"from": "武青", "to": "武清"}]
                }
            },
            {
                "custom_words": ["ollama"],
                "text": "我希望先接一下Olama吧。",
                "output": {
                    "replacements": [{"from": "Olama", "to": "ollama"}]
                }
            },
            {
                "custom_words": ["ollama"],
                "text": "我希望先接一下欧拉玛吧。",
                "output": {
                    "replacements": [{"from": "欧拉玛", "to": "ollama"}]
                }
            }
        ]
    })
    .to_string();

    CustomWordsPrompt {
        system,
        user: format!("{user}\n/no_think"),
        source_text: transcription.to_string(),
        custom_words: custom_words.to_vec(),
    }
}

fn run_qwen_helper(
    model_path: &Path,
    prompt: &CustomWordsPrompt,
) -> Result<Option<String>, String> {
    let mut helper = QWEN_HELPER
        .lock()
        .map_err(|_| "custom-word helper cache is poisoned".to_string())?;

    if helper
        .as_mut()
        .is_some_and(|process| process.child.try_wait().ok().flatten().is_some())
    {
        *helper = None;
    }

    if helper.is_none() {
        *helper = Some(start_qwen_helper()?);
    }

    let process = helper
        .as_mut()
        .ok_or_else(|| "custom-word helper was not started".to_string())?;
    let request = HelperRequest {
        model_path: model_path.to_path_buf(),
        system: prompt.system.clone(),
        user: prompt.user.clone(),
        source_text: prompt.source_text.clone(),
        custom_words: prompt.custom_words.clone(),
    };
    let request = serde_json::to_string(&request)
        .map_err(|err| format!("failed to encode custom-word helper request: {err}"))?;
    writeln!(process.stdin, "{request}")
        .and_then(|_| process.stdin.flush())
        .map_err(|err| format!("failed to send custom-word helper request: {err}"))?;

    let mut response = String::new();
    let bytes = process
        .stdout
        .read_line(&mut response)
        .map_err(|err| format!("failed to read custom-word helper response: {err}"))?;
    if bytes == 0 {
        *helper = None;
        return Err("custom-word helper exited without a response".to_string());
    }

    let response: HelperResponse = serde_json::from_str(response.trim())
        .map_err(|err| format!("custom-word helper returned invalid JSON: {err}"))?;
    if let Some(error) = response.error {
        return Err(error);
    }
    Ok(response.output.filter(|output| !output.trim().is_empty()))
}

fn start_qwen_helper() -> Result<QwenHelperProcess, String> {
    let helper_path = find_qwen_helper()
        .ok_or_else(|| format!("{} helper binary was not found", QWEN_HELPER_BIN))?;
    debug!(
        "Starting custom-word Qwen helper at {}",
        helper_path.display()
    );

    let mut child = Command::new(&helper_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| {
            format!(
                "failed to start custom-word helper {}: {err}",
                helper_path.display()
            )
        })?;

    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| "custom-word helper stdin was not available".to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "custom-word helper stdout was not available".to_string())?;

    Ok(QwenHelperProcess {
        child,
        stdin,
        stdout: BufReader::new(stdout),
    })
}

fn find_qwen_helper() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("HANDY_QWEN_HELPER").map(PathBuf::from) {
        if path.is_file() {
            return Some(path);
        }
    }

    let helper_name = if cfg!(windows) {
        format!("{QWEN_HELPER_BIN}.exe")
    } else {
        QWEN_HELPER_BIN.to_string()
    };
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let staged_resource_path = Path::new("target")
        .join("handy-qwen-resources")
        .join(&helper_name);
    let bundled_resource_path = Path::new("resources")
        .join("qwen-helper")
        .join(&helper_name);
    let candidates = [
        exe_dir.join(&helper_name),
        exe_dir.join("Resources").join(&helper_name),
        exe_dir.join("Resources").join(&staged_resource_path),
        exe_dir.join("Resources").join(&bundled_resource_path),
        exe_dir
            .parent()
            .map(|parent| parent.join(&helper_name))
            .unwrap_or_default(),
        exe_dir
            .parent()
            .map(|parent| parent.join("handy-qwen-resources").join(&helper_name))
            .unwrap_or_default(),
        exe_dir
            .parent()
            .and_then(|parent| parent.parent())
            .map(|parent| parent.join(&helper_name))
            .unwrap_or_default(),
        exe_dir
            .parent()
            .and_then(|parent| parent.parent())
            .map(|parent| {
                parent
                    .join("resources")
                    .join("qwen-helper")
                    .join(&helper_name)
            })
            .unwrap_or_default(),
        exe_dir
            .parent()
            .map(|parent| parent.join("Resources").join(&helper_name))
            .unwrap_or_default(),
        exe_dir
            .parent()
            .map(|parent| parent.join("Resources").join(&staged_resource_path))
            .unwrap_or_default(),
        exe_dir
            .parent()
            .map(|parent| parent.join("Resources").join(&bundled_resource_path))
            .unwrap_or_default(),
    ];

    candidates.into_iter().find(|path| path.is_file())
}

impl Drop for QwenHelperProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

pub(crate) fn list_custom_words_models(
    app: &AppHandle,
    selected_model_id: &str,
) -> Result<Vec<CustomWordsModelInfo>, String> {
    let app_data_dir = crate::portable::app_data_dir(app)
        .map_err(|err| format!("failed to get app data directory: {err}"))?;
    Ok(list_custom_words_models_from_root(
        app_data_dir.join("models"),
        selected_model_id,
    ))
}

fn list_custom_words_models_from_root(
    models_root: impl AsRef<Path>,
    selected_model_id: &str,
) -> Vec<CustomWordsModelInfo> {
    let selected_model_id = selected_model_id.trim();
    list_model_files(&models_root.as_ref().join(CUSTOM_WORDS_MODEL_DIR))
        .into_iter()
        .filter_map(|path| {
            let id = model_id_for_path(&path)?;
            let metadata = std::fs::metadata(&path).ok()?;
            Some(CustomWordsModelInfo {
                name: model_name_for_path(&path),
                is_selected: selected_model_id == id
                    || (selected_model_id.is_empty()
                        && find_model_file(&models_root.as_ref().join(CUSTOM_WORDS_MODEL_DIR), "")
                            .as_ref()
                            == Some(&path)),
                path: path.to_string_lossy().to_string(),
                size_bytes: metadata.len(),
                id,
            })
        })
        .collect()
}

fn find_model_file(directory: &Path, selected_model_id: &str) -> Option<PathBuf> {
    let selected_model_id = selected_model_id.trim();
    let files = list_model_files(directory);
    if selected_model_id.is_empty() {
        return files.into_iter().next();
    }

    files.into_iter().find(|path| {
        model_id_for_path(path).is_some_and(|id| id == selected_model_id)
            || path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| stem == selected_model_id)
    })
}

fn list_model_files(directory: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };

    let mut files = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("gguf"))
        })
        .collect::<Vec<_>>();
    files.sort_by(|a, b| {
        model_id_for_path(a)
            .unwrap_or_default()
            .cmp(&model_id_for_path(b).unwrap_or_default())
    });
    files
}

fn model_id_for_path(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
}

fn model_name_for_path(path: &Path) -> String {
    path.file_stem()
        .and_then(|name| name.to_str())
        .map(|name| name.replace(['_', '-'], " "))
        .unwrap_or_else(|| model_id_for_path(path).unwrap_or_else(|| CUSTOM_WORDS_MODEL_ID.into()))
}

fn eligible_custom_words(custom_words: &[String]) -> Vec<String> {
    custom_words
        .iter()
        .map(|word| word.trim())
        .filter(|word| !word.is_empty())
        .filter(|word| !is_single_cjk_char(word))
        .map(str::to_string)
        .collect()
}

fn is_single_cjk_char(word: &str) -> bool {
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    chars.next().is_none() && is_cjk_char(first)
}

fn is_cjk_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x2CEB0..=0x2EBEF
            | 0x30000..=0x3134F
            | 0x3040..=0x30FF
            | 0xAC00..=0xD7AF
    )
}

pub(crate) fn validate_model_response(
    original: &str,
    custom_words: &[String],
    response: &str,
) -> Result<Option<String>, String> {
    let custom_words = eligible_custom_words(custom_words);
    if custom_words.is_empty() {
        return Ok(None);
    }

    let payload: CorrectionResponse = serde_json::from_str(response.trim())
        .map_err(|err| format!("response is not strict JSON: {}", err))?;

    if payload.replacements.len() > MAX_REPLACEMENTS {
        return Err(format!(
            "too many replacements: {}",
            payload.replacements.len()
        ));
    }

    let allowed_words = custom_words
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let mut rebuilt = original.to_string();
    for replacement in &payload.replacements {
        if replacement.from.is_empty() {
            return Err("replacement.from is empty".to_string());
        }
        if !allowed_words.contains(replacement.to.as_str()) {
            return Err(format!(
                "replacement.to '{}' is not in custom_words",
                replacement.to
            ));
        }
        if !rebuilt.contains(&replacement.from) {
            return Err(format!(
                "replacement.from '{}' does not occur in the current text",
                replacement.from
            ));
        }
        rebuilt = rebuilt.replace(&replacement.from, &replacement.to);
    }

    if payload.text != rebuilt {
        return Err("response text changes content outside declared replacements".to_string());
    }

    if payload.text == original {
        return Ok(None);
    }

    Ok(Some(payload.text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::get_default_settings;
    use std::fs;
    use tempfile::TempDir;

    struct StaticRuntime {
        output: Option<&'static str>,
    }

    impl CustomWordsCorrectionRuntime for StaticRuntime {
        fn generate(&self, _prompt: &CustomWordsPrompt) -> Result<Option<String>, String> {
            Ok(self.output.map(str::to_string))
        }
    }

    fn words(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    fn settings_with_words(items: &[&str]) -> AppSettings {
        let mut settings = get_default_settings();
        settings.custom_words = words(items);
        settings
    }

    #[test]
    fn accepts_qwen_name_correction() {
        let result = validate_model_response(
            "罗德群今天回家吗?",
            &words(&["罗泽群"]),
            r#"{"text":"罗泽群今天回家吗?","replacements":[{"from":"罗德群","to":"罗泽群"}]}"#,
        )
        .unwrap();

        assert_eq!(result.as_deref(), Some("罗泽群今天回家吗?"));
    }

    #[test]
    fn accepts_exact_hit_as_no_change() {
        let result = validate_model_response(
            "我今天去了阿里巴巴开会",
            &words(&["阿里巴巴"]),
            r#"{"text":"我今天去了阿里巴巴开会","replacements":[]}"#,
        )
        .unwrap();

        assert_eq!(result, None);
    }

    #[test]
    fn accepts_qwen_two_char_place_correction() {
        let result = validate_model_response(
            "武青在天津的西北边。",
            &words(&["武清"]),
            r#"{"text":"武清在天津的西北边。","replacements":[{"from":"武青","to":"武清"}]}"#,
        )
        .unwrap();

        assert_eq!(result.as_deref(), Some("武清在天津的西北边。"));
    }

    #[test]
    fn accepts_sentence_middle_and_punctuation_boundary() {
        let result = validate_model_response(
            "他说：阿里巴吧，明天见。",
            &words(&["阿里巴巴"]),
            r#"{"text":"他说：阿里巴巴，明天见。","replacements":[{"from":"阿里巴吧","to":"阿里巴巴"}]}"#,
        )
        .unwrap();

        assert_eq!(result.as_deref(), Some("他说：阿里巴巴，明天见。"));
    }

    #[test]
    fn accepts_explicit_simplified_to_traditional_replacement() {
        let result = validate_model_response(
            "我申请了台湾大学",
            &words(&["臺灣大學"]),
            r#"{"text":"我申请了臺灣大學","replacements":[{"from":"台湾大学","to":"臺灣大學"}]}"#,
        )
        .unwrap();

        assert_eq!(result.as_deref(), Some("我申请了臺灣大學"));
    }

    #[test]
    fn accepts_latin_custom_word_spelling_correction() {
        let result = validate_model_response(
            "我希望先接一下Olama吧。",
            &words(&["ollama"]),
            r#"{"text":"我希望先接一下ollama吧。","replacements":[{"from":"Olama","to":"ollama"}]}"#,
        )
        .unwrap();

        assert_eq!(result.as_deref(), Some("我希望先接一下ollama吧。"));
    }

    #[test]
    fn accepts_cjk_transliteration_to_latin_custom_word() {
        let result = validate_model_response(
            "我希望先接一下欧拉玛吧。",
            &words(&["ollama"]),
            r#"{"text":"我希望先接一下ollama吧。","replacements":[{"from":"欧拉玛","to":"ollama"}]}"#,
        )
        .unwrap();

        assert_eq!(result.as_deref(), Some("我希望先接一下ollama吧。"));
    }

    #[test]
    fn accepts_replacing_every_occurrence_of_declared_span() {
        let result = validate_model_response(
            "欧拉玛和欧拉玛都要改。",
            &words(&["ollama"]),
            r#"{"text":"ollama和ollama都要改。","replacements":[{"from":"欧拉玛","to":"ollama"}]}"#,
        )
        .unwrap();

        assert_eq!(result.as_deref(), Some("ollama和ollama都要改。"));
    }

    #[test]
    fn prompt_teaches_latin_and_transliteration_corrections() {
        let prompt = build_prompt("我希望先接一下欧拉玛吧。", &words(&["ollama"]));

        assert!(prompt.system.contains("CJK transliteration"));
        assert!(prompt.user.contains("\"from\":\"Olama\""));
        assert!(prompt.user.contains("\"from\":\"欧拉玛\""));
        assert!(prompt.user.contains("\"to\":\"ollama\""));
    }

    #[test]
    fn runtime_abstraction_accepts_valid_model_json() {
        let settings = settings_with_words(&["罗泽群"]);
        let runtime = StaticRuntime {
            output: Some(
                r#"{"text":"罗泽群今天回家吗?","replacements":[{"from":"罗德群","to":"罗泽群"}]}"#,
            ),
        };

        let result = correct_custom_words_with_runtime(&runtime, &settings, "罗德群今天回家吗?");

        assert_eq!(result.as_deref(), Some("罗泽群今天回家吗?"));
    }

    #[test]
    fn runtime_abstraction_rejects_invalid_model_json() {
        let settings = settings_with_words(&["武清"]);
        let runtime = StaticRuntime {
            output: Some("武清在天津的西北边。"),
        };

        let result = correct_custom_words_with_runtime(&runtime, &settings, "武青在天津的西北边。");

        assert_eq!(result, None);
    }

    #[test]
    fn resource_manager_marks_missing_model() {
        let temp = TempDir::new().unwrap();
        let resource = CustomWordsModelResource::from_models_root_with_selection(temp.path(), "");

        assert_eq!(resource.status, CustomWordsModelStatus::Missing);
        assert_eq!(resource.model_path, None);
        assert!(resource.directory.ends_with(CUSTOM_WORDS_MODEL_DIR));
    }

    #[test]
    fn resource_manager_finds_gguf_model() {
        let temp = TempDir::new().unwrap();
        let model_dir = temp.path().join(CUSTOM_WORDS_MODEL_DIR);
        fs::create_dir_all(&model_dir).unwrap();
        let model_path = model_dir.join("qwen3-0.6b.gguf");
        fs::write(&model_path, b"test").unwrap();

        let resource = CustomWordsModelResource::from_models_root_with_selection(temp.path(), "");

        assert_eq!(resource.status, CustomWordsModelStatus::Installed);
        assert_eq!(resource.model_path.as_deref(), Some(model_path.as_path()));
    }

    #[test]
    fn resource_manager_uses_selected_gguf_model() {
        let temp = TempDir::new().unwrap();
        let model_dir = temp.path().join(CUSTOM_WORDS_MODEL_DIR);
        fs::create_dir_all(&model_dir).unwrap();
        let default_path = model_dir.join("a-default.gguf");
        let selected_path = model_dir.join("z-selected.gguf");
        fs::write(&default_path, b"default").unwrap();
        fs::write(&selected_path, b"selected").unwrap();

        let resource = CustomWordsModelResource::from_models_root_with_selection(
            temp.path(),
            "z-selected.gguf",
        );

        assert_eq!(resource.status, CustomWordsModelStatus::Installed);
        assert_eq!(
            resource.model_path.as_deref(),
            Some(selected_path.as_path())
        );
    }

    #[test]
    fn resource_manager_does_not_fallback_when_selected_model_is_missing() {
        let temp = TempDir::new().unwrap();
        let model_dir = temp.path().join(CUSTOM_WORDS_MODEL_DIR);
        fs::create_dir_all(&model_dir).unwrap();
        fs::write(model_dir.join("qwen3-0.6b.gguf"), b"test").unwrap();

        let resource =
            CustomWordsModelResource::from_models_root_with_selection(temp.path(), "missing.gguf");

        assert_eq!(resource.status, CustomWordsModelStatus::Missing);
        assert_eq!(resource.model_path, None);
    }

    #[test]
    fn resource_manager_lists_selectable_gguf_models() {
        let temp = TempDir::new().unwrap();
        let model_dir = temp.path().join(CUSTOM_WORDS_MODEL_DIR);
        fs::create_dir_all(&model_dir).unwrap();
        fs::write(model_dir.join("b.gguf"), b"test").unwrap();
        fs::write(model_dir.join("a.gguf"), b"test").unwrap();
        fs::write(model_dir.join("notes.txt"), b"ignored").unwrap();

        let models = list_custom_words_models_from_root(temp.path(), "b.gguf");

        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a.gguf", "b.gguf"]
        );
        assert_eq!(models.iter().filter(|model| model.is_selected).count(), 1);
        assert!(models
            .iter()
            .any(|model| model.id == "b.gguf" && model.is_selected));
    }

    #[test]
    #[ignore = "requires a local Qwen3-0.6B GGUF model resource"]
    fn qwen_runtime_corrects_reported_examples_when_model_present() {
        let models_root = std::env::var_os("HANDY_QWEN_MODEL_ROOT")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|home| {
                    PathBuf::from(home)
                        .join("Library")
                        .join("Application Support")
                        .join("com.pais.handy")
                        .join("models")
                })
            })
            .expect("HOME or HANDY_QWEN_MODEL_ROOT must be set");
        let resource = CustomWordsModelResource::from_models_root_with_selection(models_root, "");
        assert_eq!(resource.status, CustomWordsModelStatus::Installed);

        let runtime = SelfManagedQwenRuntime::new(resource);

        let settings = settings_with_words(&["罗泽群"]);
        let custom_words = eligible_custom_words(&settings.custom_words);
        let prompt = build_prompt("罗德群今天回家吗?", &custom_words);
        let raw = runtime
            .generate(&prompt)
            .expect("helper should produce a response")
            .expect("helper response should not be empty");
        eprintln!("qwen raw name correction: {raw}");
        let result = validate_model_response("罗德群今天回家吗?", &custom_words, &raw).unwrap();
        assert_eq!(result.as_deref(), Some("罗泽群今天回家吗?"));

        let settings = settings_with_words(&["武清"]);
        let custom_words = eligible_custom_words(&settings.custom_words);
        let prompt = build_prompt("武青在天津的西北边。", &custom_words);
        let raw = runtime
            .generate(&prompt)
            .expect("helper should produce a response")
            .expect("helper response should not be empty");
        eprintln!("qwen raw place correction: {raw}");
        let result = validate_model_response("武青在天津的西北边。", &custom_words, &raw).unwrap();
        assert_eq!(result.as_deref(), Some("武清在天津的西北边。"));
    }

    #[test]
    fn rejects_replacement_to_outside_custom_words() {
        let err = validate_model_response(
            "罗德群今天回家吗?",
            &words(&["罗泽群"]),
            r#"{"text":"罗志群今天回家吗?","replacements":[{"from":"罗德群","to":"罗志群"}]}"#,
        )
        .unwrap_err();

        assert!(err.contains("not in custom_words"));
    }

    #[test]
    fn rejects_undeclared_free_rewrite() {
        let err = validate_model_response(
            "罗德群今天回家吗?",
            &words(&["罗泽群"]),
            r#"{"text":"罗泽群今天要回家吗?","replacements":[{"from":"罗德群","to":"罗泽群"}]}"#,
        )
        .unwrap_err();

        assert!(err.contains("outside declared replacements"));
    }

    #[test]
    fn rejects_non_json_output() {
        let err = validate_model_response(
            "武青在天津的西北边。",
            &words(&["武清"]),
            "武清在天津的西北边。",
        )
        .unwrap_err();

        assert!(err.contains("strict JSON"));
    }

    #[test]
    fn rejects_single_cjk_custom_word_replacement() {
        let result = validate_model_response(
            "我今天去了阿里巴吧开会",
            &words(&["巴"]),
            r#"{"text":"我今天去了阿里巴巴开会","replacements":[{"from":"吧","to":"巴"}]}"#,
        )
        .unwrap();

        assert_eq!(result, None);
    }
}
