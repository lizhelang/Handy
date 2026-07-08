use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr::null_mut;
use std::sync::{Arc, Mutex};

use inputia_core::{
    AppContext, AppPolicy, Candidate, CandidateCommit, CharacterWidthPreference, ChineseEngine,
    CoreSettings, InputMode, InputOutcome, InputiaCore, Key, MemorySource, PrivacyDecision,
    PunctuationPreference, SqliteMemory,
};
use inputia_rime::{RimeEngine, RimeEngineConfig};
use inputia_settings::InputiaSettings;
use serde::Serialize;

const KEY_BACKSPACE: c_int = 1;
const KEY_ESCAPE: c_int = 2;
const KEY_SPACE: c_int = 3;
const KEY_SHIFT: c_int = 4;
const KEY_PAGE_DOWN: c_int = 5;
const KEY_PAGE_UP: c_int = 6;
const KEY_ENTER: c_int = 7;
const KEY_TOGGLE_PUNCTUATION: c_int = 8;
const KEY_TOGGLE_CHARACTER_WIDTH: c_int = 9;
const KEY_TOGGLE_INPUT_MODE: c_int = 10;
const INPUT_MODE_ENGLISH: c_int = 1;
const INPUT_MODE_CHINESE: c_int = 2;
const SOURCE_TYPED: c_int = 1;
const SOURCE_VOICE: c_int = 2;
const SOURCE_CLIPBOARD: c_int = 3;

pub struct InputiaSession {
    core: InputiaCore<RankedRimeEngine>,
    memory: Option<Arc<Mutex<SqliteMemory>>>,
    context: AppContext,
}

struct SessionOptions {
    rime: RimeEngineConfig,
    core: CoreSettings,
    memory_db_path: Option<String>,
    policy: AppPolicy,
}

struct RankedRimeEngine {
    rime: RimeEngine,
    memory: Option<Arc<Mutex<SqliteMemory>>>,
}

impl ChineseEngine for RankedRimeEngine {
    fn candidates(&self, composing: &str) -> Vec<Candidate> {
        let candidates = self.rime.candidates(composing);
        let Some(memory) = &self.memory else {
            return candidates;
        };

        let Ok(memory) = memory.lock() else {
            return candidates;
        };
        memory
            .rank_candidates(candidates.clone())
            .unwrap_or(candidates)
    }

    fn select_candidate(
        &self,
        composing: &str,
        candidate: &Candidate,
        page: usize,
        page_index: usize,
    ) -> Option<CandidateCommit> {
        let snapshot = self
            .rime
            .select_live_candidate(composing, candidate, page, page_index)
            .ok()?;
        let candidates = if snapshot.input.is_empty() {
            Vec::new()
        } else {
            self.candidates(&snapshot.input)
        };
        if let Some(commit) = snapshot.commit.filter(|commit| !commit.is_empty()) {
            return Some(CandidateCommit::new(
                commit,
                snapshot.input,
                snapshot.page_no.max(0) as usize,
                candidates,
            ));
        }
        if candidates.is_empty() {
            return Some(CandidateCommit::new(
                candidate.text.clone(),
                "",
                0,
                Vec::new(),
            ));
        }
        Some(CandidateCommit::preedit(
            snapshot.preedit,
            snapshot.input,
            snapshot.page_no.max(0) as usize,
            candidates,
        ))
    }
}

#[no_mangle]
pub extern "C" fn inputia_session_new_luna_pinyin_simp(
    user_data_dir: *const c_char,
    candidate_page_size: usize,
) -> *mut InputiaSession {
    new_session("luna_pinyin_simp", user_data_dir, candidate_page_size, None)
}

#[no_mangle]
pub extern "C" fn inputia_session_new_luna_pinyin_simp_with_memory(
    user_data_dir: *const c_char,
    memory_db_path: *const c_char,
    candidate_page_size: usize,
) -> *mut InputiaSession {
    let Some(memory_db_path) = (unsafe { optional_c_string(memory_db_path) }) else {
        return null_mut();
    };
    new_session(
        "luna_pinyin_simp",
        user_data_dir,
        candidate_page_size,
        Some(memory_db_path),
    )
}

#[no_mangle]
pub extern "C" fn inputia_session_new_with_schema(
    schema_id: *const c_char,
    user_data_dir: *const c_char,
    candidate_page_size: usize,
) -> *mut InputiaSession {
    let Some(schema_id) = (unsafe { optional_c_string(schema_id) }) else {
        return null_mut();
    };
    new_session(&schema_id, user_data_dir, candidate_page_size, None)
}

#[no_mangle]
pub extern "C" fn inputia_session_new_with_paths(
    schema_id: *const c_char,
    dylib_path: *const c_char,
    shared_data_dir: *const c_char,
    user_data_dir: *const c_char,
    candidate_page_size: usize,
) -> *mut InputiaSession {
    let Some(schema_id) = (unsafe { optional_c_string(schema_id) }) else {
        return null_mut();
    };
    let Some(dylib_path) = (unsafe { optional_c_string(dylib_path) }) else {
        return null_mut();
    };
    let Some(shared_data_dir) = (unsafe { optional_c_string(shared_data_dir) }) else {
        return null_mut();
    };
    let Some(user_data_dir) = (unsafe { optional_c_string(user_data_dir) }) else {
        return null_mut();
    };

    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir)
        .with_schema(schema_id)
        .with_dylib_path(dylib_path)
        .with_shared_data_dir(shared_data_dir);
    new_session_with_options(SessionOptions {
        rime: config,
        core: core_settings(
            candidate_page_size,
            true,
            PunctuationPreference::EnglishInChinese,
            CharacterWidthPreference::HalfWidth,
        ),
        memory_db_path: None,
        policy: AppPolicy::default(),
    })
}

#[no_mangle]
pub extern "C" fn inputia_session_new_from_settings(
    settings_path: *const c_char,
) -> *mut InputiaSession {
    let Some(settings_path) = (unsafe { optional_c_string(settings_path) }) else {
        return null_mut();
    };
    let Ok(settings) = InputiaSettings::load_or_create(&settings_path) else {
        return null_mut();
    };
    let Some(options) = session_options_from_settings(settings) else {
        return null_mut();
    };
    new_session_with_options(options)
}

#[no_mangle]
pub extern "C" fn inputia_session_free(session: *mut InputiaSession) {
    if !session.is_null() {
        unsafe { drop(Box::from_raw(session)) };
    }
}

#[no_mangle]
pub extern "C" fn inputia_session_handle_char(
    session: *mut InputiaSession,
    unicode_scalar: u32,
) -> *mut c_char {
    let Some(ch) = char::from_u32(unicode_scalar) else {
        return error_json("invalid Unicode scalar");
    };
    with_session(session, |session| session.core.handle_key(Key::Char(ch)))
}

#[no_mangle]
pub extern "C" fn inputia_session_handle_digit(
    session: *mut InputiaSession,
    digit: u8,
) -> *mut c_char {
    with_session(session, |session| {
        session.core.handle_key(Key::Digit(digit))
    })
}

#[no_mangle]
pub extern "C" fn inputia_session_handle_special(
    session: *mut InputiaSession,
    special_key: c_int,
) -> *mut c_char {
    let key = match special_key {
        KEY_BACKSPACE => Key::Backspace,
        KEY_ESCAPE => Key::Escape,
        KEY_SPACE => Key::Space,
        KEY_SHIFT => Key::Shift,
        KEY_PAGE_DOWN => Key::PageDown,
        KEY_PAGE_UP => Key::PageUp,
        KEY_ENTER => Key::Enter,
        KEY_TOGGLE_PUNCTUATION => Key::TogglePunctuation,
        KEY_TOGGLE_CHARACTER_WIDTH => Key::ToggleCharacterWidth,
        KEY_TOGGLE_INPUT_MODE => Key::ToggleInputMode,
        _ => return error_json("unknown special key"),
    };
    with_session(session, |session| session.core.handle_key(key))
}

#[no_mangle]
pub extern "C" fn inputia_session_snapshot(session: *mut InputiaSession) -> *mut c_char {
    if session.is_null() {
        return error_json("session is null");
    }
    let session = unsafe { &mut *session };
    outcome_json(OutputEnvelope::ok(None, false, session.core.snapshot()))
}

#[no_mangle]
pub extern "C" fn inputia_session_set_input_mode(
    session: *mut InputiaSession,
    input_mode: c_int,
) -> *mut c_char {
    let mode = match input_mode {
        INPUT_MODE_ENGLISH => InputMode::English,
        INPUT_MODE_CHINESE => InputMode::Chinese,
        _ => return error_json("unknown input mode"),
    };
    with_session(session, |session| session.core.set_mode(mode))
}

#[no_mangle]
pub extern "C" fn inputia_session_set_app_context(
    session: *mut InputiaSession,
    bundle_id: *const c_char,
) -> *mut c_char {
    if session.is_null() {
        return learning_json(LearningEnvelope::error("session is null"));
    }
    let Some(bundle_id) = (unsafe { optional_c_string(bundle_id) }) else {
        return learning_json(LearningEnvelope::error("bundle id is null"));
    };
    let session = unsafe { &mut *session };
    session.context = AppContext::new(bundle_id);
    learning_json(LearningEnvelope::context_set())
}

#[no_mangle]
pub extern "C" fn inputia_session_set_app_context_with_window(
    session: *mut InputiaSession,
    bundle_id: *const c_char,
    window_title: *const c_char,
) -> *mut c_char {
    if session.is_null() {
        return learning_json(LearningEnvelope::error("session is null"));
    }
    let Some(bundle_id) = (unsafe { optional_c_string(bundle_id) }) else {
        return learning_json(LearningEnvelope::error("bundle id is null"));
    };
    let window_title = unsafe { optional_c_string(window_title) }
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());

    let session = unsafe { &mut *session };
    session.context = AppContext::new(bundle_id).with_window_title(window_title);
    learning_json(LearningEnvelope::context_set())
}

#[no_mangle]
pub extern "C" fn inputia_session_learn(
    session: *mut InputiaSession,
    source: c_int,
    text: *const c_char,
    bundle_id: *const c_char,
) -> *mut c_char {
    if session.is_null() {
        return learning_json(LearningEnvelope::error("session is null"));
    }
    let Some(source) = memory_source(source) else {
        return learning_json(LearningEnvelope::error("unknown memory source"));
    };
    let Some(text) = (unsafe { optional_c_string(text) }) else {
        return learning_json(LearningEnvelope::error("text is null"));
    };
    let Some(bundle_id) = (unsafe { optional_c_string(bundle_id) }) else {
        return learning_json(LearningEnvelope::error("bundle id is null"));
    };

    let session = unsafe { &mut *session };
    let Some(memory) = &session.memory else {
        return learning_json(LearningEnvelope::error("memory is not enabled"));
    };
    let Ok(mut memory) = memory.lock() else {
        return learning_json(LearningEnvelope::error("memory lock is poisoned"));
    };
    let fallback_context;
    let context = if session.context.bundle_id == bundle_id {
        &session.context
    } else {
        fallback_context = AppContext::new(bundle_id);
        &fallback_context
    };
    match memory.learn(source, text, context) {
        Ok(outcome) => learning_json(LearningEnvelope::from_outcome(outcome)),
        Err(_) => learning_json(LearningEnvelope::error("failed to learn term")),
    }
}

#[no_mangle]
pub extern "C" fn inputia_session_import_handy_history(
    session: *mut InputiaSession,
    history_db_path: *const c_char,
    bundle_id: *const c_char,
    limit: usize,
) -> *mut c_char {
    if session.is_null() {
        return import_json(ImportEnvelope::error("session is null"));
    }
    let Some(history_db_path) = (unsafe { optional_c_string(history_db_path) }) else {
        return import_json(ImportEnvelope::error("history database path is null"));
    };
    let Some(bundle_id) = (unsafe { optional_c_string(bundle_id) }) else {
        return import_json(ImportEnvelope::error("bundle id is null"));
    };

    let session = unsafe { &mut *session };
    let Some(memory) = &session.memory else {
        return import_json(ImportEnvelope::error("memory is not enabled"));
    };
    let Ok(mut memory) = memory.lock() else {
        return import_json(ImportEnvelope::error("memory lock is poisoned"));
    };

    match memory.import_handy_history(history_db_path, &AppContext::new(bundle_id), limit) {
        Ok(imported) => import_json(ImportEnvelope::ok(imported)),
        Err(_) => import_json(ImportEnvelope::error("failed to import Handy history")),
    }
}

#[no_mangle]
pub extern "C" fn inputia_session_import_handy_clipboard(
    session: *mut InputiaSession,
    clipboard_db_path: *const c_char,
    bundle_id: *const c_char,
    limit: usize,
) -> *mut c_char {
    if session.is_null() {
        return import_json(ImportEnvelope::error("session is null"));
    }
    let Some(clipboard_db_path) = (unsafe { optional_c_string(clipboard_db_path) }) else {
        return import_json(ImportEnvelope::error("clipboard database path is null"));
    };
    let Some(bundle_id) = (unsafe { optional_c_string(bundle_id) }) else {
        return import_json(ImportEnvelope::error("bundle id is null"));
    };

    let session = unsafe { &mut *session };
    let Some(memory) = &session.memory else {
        return import_json(ImportEnvelope::error("memory is not enabled"));
    };
    let Ok(mut memory) = memory.lock() else {
        return import_json(ImportEnvelope::error("memory lock is poisoned"));
    };

    match memory.import_handy_clipboard(clipboard_db_path, &AppContext::new(bundle_id), limit) {
        Ok(imported) => import_json(ImportEnvelope::ok(imported)),
        Err(_) => import_json(ImportEnvelope::error("failed to import Handy clipboard")),
    }
}

#[no_mangle]
pub extern "C" fn inputia_session_voice_hotwords(
    session: *mut InputiaSession,
    limit: usize,
) -> *mut c_char {
    if session.is_null() {
        return hotwords_json(HotwordsEnvelope::error("session is null"));
    }
    let session = unsafe { &mut *session };
    let Some(memory) = &session.memory else {
        return hotwords_json(HotwordsEnvelope::error("memory is not enabled"));
    };
    let Ok(memory) = memory.lock() else {
        return hotwords_json(HotwordsEnvelope::error("memory lock is poisoned"));
    };
    match memory.voice_hotwords(limit) {
        Ok(hotwords) => hotwords_json(HotwordsEnvelope::ok(hotwords)),
        Err(_) => hotwords_json(HotwordsEnvelope::error("failed to load hotwords")),
    }
}

#[no_mangle]
pub extern "C" fn inputia_session_clipboard_candidates(
    session: *mut InputiaSession,
    limit: usize,
) -> *mut c_char {
    if session.is_null() {
        return candidate_list_json(CandidateListEnvelope::error("session is null"));
    }
    let session = unsafe { &mut *session };
    let Some(memory) = &session.memory else {
        return candidate_list_json(CandidateListEnvelope::error("memory is not enabled"));
    };
    let Ok(memory) = memory.lock() else {
        return candidate_list_json(CandidateListEnvelope::error("memory lock is poisoned"));
    };
    match memory.clipboard_candidates(limit) {
        Ok(candidates) => candidate_list_json(CandidateListEnvelope::ok(candidates)),
        Err(_) => candidate_list_json(CandidateListEnvelope::error(
            "failed to load clipboard candidates",
        )),
    }
}

#[no_mangle]
pub extern "C" fn inputia_session_completion_candidates(
    session: *mut InputiaSession,
    prefix: *const c_char,
    limit: usize,
) -> *mut c_char {
    if session.is_null() {
        return candidate_list_json(CandidateListEnvelope::error("session is null"));
    }
    let Some(prefix) = (unsafe { optional_c_string(prefix) }) else {
        return candidate_list_json(CandidateListEnvelope::error("prefix is null"));
    };
    let session = unsafe { &mut *session };
    let Some(memory) = &session.memory else {
        return candidate_list_json(CandidateListEnvelope::error("memory is not enabled"));
    };
    let Ok(memory) = memory.lock() else {
        return candidate_list_json(CandidateListEnvelope::error("memory lock is poisoned"));
    };
    match memory.english_completion_candidates(&prefix, limit) {
        Ok(candidates) => candidate_list_json(CandidateListEnvelope::ok(candidates)),
        Err(_) => candidate_list_json(CandidateListEnvelope::error(
            "failed to load completion candidates",
        )),
    }
}

#[no_mangle]
pub extern "C" fn inputia_string_free(value: *mut c_char) {
    if !value.is_null() {
        unsafe { drop(CString::from_raw(value)) };
    }
}

fn new_session(
    schema_id: &str,
    user_data_dir: *const c_char,
    candidate_page_size: usize,
    memory_db_path: Option<String>,
) -> *mut InputiaSession {
    let Some(user_data_dir) = (unsafe { optional_c_string(user_data_dir) }) else {
        return null_mut();
    };
    let config = apply_default_shared_data_dir(
        RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir).with_schema(schema_id),
    );
    new_session_with_options(SessionOptions {
        rime: config,
        core: core_settings(
            candidate_page_size,
            true,
            PunctuationPreference::EnglishInChinese,
            CharacterWidthPreference::HalfWidth,
        ),
        memory_db_path,
        policy: AppPolicy::default(),
    })
}

fn new_session_with_options(options: SessionOptions) -> *mut InputiaSession {
    let Ok(engine) = RimeEngine::open(options.rime) else {
        return null_mut();
    };
    let memory = match options.memory_db_path {
        Some(path) => {
            if let Some(parent) = std::path::Path::new(&path).parent() {
                if std::fs::create_dir_all(parent).is_err() {
                    return null_mut();
                }
            }
            match SqliteMemory::open(path, options.policy) {
                Ok(memory) => Some(Arc::new(Mutex::new(memory))),
                Err(_) => return null_mut(),
            }
        }
        None => None,
    };
    let ranked_engine = RankedRimeEngine {
        rime: engine,
        memory: memory.clone(),
    };
    let core = InputiaCore::new(options.core, ranked_engine);
    Box::into_raw(Box::new(InputiaSession {
        core,
        memory,
        context: AppContext::new("dev.inputia.host"),
    }))
}

fn session_options_from_settings(settings: InputiaSettings) -> Option<SessionOptions> {
    let rime_user_data_dir = settings.rime_user_data_dir?;
    let spelling_correction =
        effective_spelling_correction(&settings.schema_id, settings.spelling_correction_enabled);
    let (schema_id, output_options) =
        effective_rime_script_config(&settings.schema_id, &settings.chinese_script);
    let mut rime = RimeEngineConfig::squirrel_luna_pinyin_simp(rime_user_data_dir)
        .with_schema(schema_id)
        .with_output_options(output_options)
        .with_spelling_correction(spelling_correction);
    if let Some(path) = settings.rime_dylib_path {
        rime = rime.with_dylib_path(path);
    }
    if let Some(path) = settings
        .rime_shared_data_dir
        .filter(|path| path.exists())
        .or_else(default_inputia_shared_data_dir)
    {
        rime = rime.with_shared_data_dir(path);
    }
    let memory_db_path = if settings.memory_enabled && settings.privacy_learning_enabled {
        settings
            .memory_db_path
            .map(|path| path.to_string_lossy().into_owned())
    } else {
        None
    };
    let punctuation_preference = match settings.punctuation_preference {
        inputia_settings::PunctuationPreference::FollowInputMode => {
            PunctuationPreference::FollowInputMode
        }
        inputia_settings::PunctuationPreference::EnglishInChinese => {
            PunctuationPreference::EnglishInChinese
        }
    };
    let character_width_preference = match settings.character_width_preference {
        inputia_settings::CharacterWidthPreference::HalfWidth => {
            CharacterWidthPreference::HalfWidth
        }
        inputia_settings::CharacterWidthPreference::FullWidth => {
            CharacterWidthPreference::FullWidth
        }
    };

    Some(SessionOptions {
        rime,
        core: core_settings(
            settings.candidate_page_size,
            settings.shift_toggle_enabled,
            punctuation_preference,
            character_width_preference,
        ),
        memory_db_path,
        policy: AppPolicy::with_sensitive_bundle_ids(settings.sensitive_bundle_ids),
    })
}

fn effective_rime_script_config(
    schema_id: &str,
    chinese_script: &inputia_settings::ChineseScript,
) -> (String, Vec<(String, bool)>) {
    match (schema_id, chinese_script) {
        ("luna_pinyin_simp", inputia_settings::ChineseScript::Traditional) => (
            "luna_pinyin".to_string(),
            vec![
                ("simplification".to_string(), false),
                ("zh_hans".to_string(), false),
                ("zh_hant".to_string(), true),
            ],
        ),
        ("luna_pinyin", inputia_settings::ChineseScript::Simplified) => (
            "luna_pinyin".to_string(),
            vec![
                ("simplification".to_string(), true),
                ("zh_hant".to_string(), false),
                ("zh_hans".to_string(), true),
            ],
        ),
        ("luna_pinyin", inputia_settings::ChineseScript::Traditional) => (
            "luna_pinyin".to_string(),
            vec![
                ("simplification".to_string(), false),
                ("zh_hans".to_string(), false),
                ("zh_hant".to_string(), true),
            ],
        ),
        ("guobiao_bispell", inputia_settings::ChineseScript::Traditional) => (
            "guobiao_bispell".to_string(),
            vec![
                ("simplification".to_string(), false),
                ("trad_tw".to_string(), true),
            ],
        ),
        (_, inputia_settings::ChineseScript::Simplified) => (
            schema_id.to_string(),
            vec![("simplification".to_string(), true)],
        ),
        (_, inputia_settings::ChineseScript::Traditional) => (
            schema_id.to_string(),
            vec![("simplification".to_string(), false)],
        ),
    }
}

fn effective_spelling_correction(schema_id: &str, requested: bool) -> bool {
    requested && is_full_pinyin_schema(schema_id)
}

fn is_full_pinyin_schema(schema_id: &str) -> bool {
    matches!(
        schema_id,
        "luna_pinyin"
            | "luna_pinyin_simp"
            | "luna_pinyin_tw"
            | "luna_pinyin_fluency"
            | "luna_quanpin"
    )
}

fn default_inputia_shared_data_dir() -> Option<std::path::PathBuf> {
    current_bundle_rime_data_dir()
        .into_iter()
        .chain(
            std::env::var("INPUTIA_RIME_SHARED_DATA_DIR")
                .ok()
                .map(std::path::PathBuf::from),
        )
        .chain(
            [
                "/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/RimeData",
                "/Library/Input Methods/IputiaInputMethod.app/Contents/Resources/RimeData",
            ]
            .into_iter()
            .map(std::path::PathBuf::from),
        )
        .chain(std::iter::once(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../macos/InputiaInputMethod/build/RimeData"),
        ))
        .find(|path| path.exists())
}

fn apply_default_shared_data_dir(config: RimeEngineConfig) -> RimeEngineConfig {
    if let Some(path) = default_inputia_shared_data_dir() {
        config.with_shared_data_dir(path)
    } else {
        config
    }
}

fn current_bundle_rime_data_dir() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let macos_dir = exe.parent()?;
    let contents_dir = macos_dir.parent()?;
    Some(contents_dir.join("Resources").join("RimeData"))
}

fn core_settings(
    candidate_page_size: usize,
    shift_toggle_enabled: bool,
    punctuation_preference: PunctuationPreference,
    character_width_preference: CharacterWidthPreference,
) -> CoreSettings {
    CoreSettings {
        candidate_page_size: candidate_page_size.max(1),
        shift_toggle_enabled,
        punctuation_preference,
        character_width_preference,
    }
}

fn with_session(
    session: *mut InputiaSession,
    handle: impl FnOnce(&mut InputiaSession) -> InputOutcome,
) -> *mut c_char {
    if session.is_null() {
        return error_json("session is null");
    }
    let session = unsafe { &mut *session };
    let outcome = handle(session);
    learn_committed_text(session, &outcome);
    outcome_json(OutputEnvelope::from_outcome(outcome))
}

fn learn_committed_text(session: &mut InputiaSession, outcome: &InputOutcome) {
    let Some(commit) = outcome.commit.as_ref() else {
        return;
    };
    let Some(memory) = &session.memory else {
        return;
    };
    let Ok(mut memory) = memory.lock() else {
        return;
    };
    let _ = memory.learn(MemorySource::Typed, commit, &session.context);
}

unsafe fn optional_c_string(value: *const c_char) -> Option<String> {
    if value.is_null() {
        return None;
    }
    Some(CStr::from_ptr(value).to_string_lossy().into_owned())
}

fn outcome_json(envelope: OutputEnvelope) -> *mut c_char {
    let json = serde_json::to_string(&envelope).unwrap_or_else(|_| {
        r#"{"ok":false,"error":"failed to serialize outcome","consumed":false,"commit":null,"mode":"English","composing":"","page":0,"visible_candidates":[]}"#
            .to_string()
    });
    CString::new(json)
        .map(CString::into_raw)
        .unwrap_or(null_mut())
}

fn error_json(message: &'static str) -> *mut c_char {
    outcome_json(OutputEnvelope::error(message))
}

fn learning_json(envelope: LearningEnvelope) -> *mut c_char {
    string_json(&envelope)
}

fn hotwords_json(envelope: HotwordsEnvelope) -> *mut c_char {
    string_json(&envelope)
}

fn candidate_list_json(envelope: CandidateListEnvelope) -> *mut c_char {
    string_json(&envelope)
}

fn import_json(envelope: ImportEnvelope) -> *mut c_char {
    string_json(&envelope)
}

fn string_json(envelope: &impl Serialize) -> *mut c_char {
    let json = serde_json::to_string(envelope)
        .unwrap_or_else(|_| r#"{"ok":false,"error":"failed to serialize response"}"#.to_string());
    CString::new(json)
        .map(CString::into_raw)
        .unwrap_or(null_mut())
}

fn memory_source(source: c_int) -> Option<MemorySource> {
    match source {
        SOURCE_TYPED => Some(MemorySource::Typed),
        SOURCE_VOICE => Some(MemorySource::Voice),
        SOURCE_CLIPBOARD => Some(MemorySource::Clipboard),
        _ => None,
    }
}

#[derive(Serialize)]
struct OutputEnvelope {
    ok: bool,
    error: Option<&'static str>,
    consumed: bool,
    commit: Option<String>,
    mode: &'static str,
    composing: String,
    page: usize,
    visible_candidates: Vec<CandidateEnvelope>,
}

impl OutputEnvelope {
    fn from_outcome(outcome: InputOutcome) -> Self {
        Self::ok(outcome.commit, outcome.consumed, outcome.snapshot)
    }

    fn ok(commit: Option<String>, consumed: bool, snapshot: inputia_core::InputSnapshot) -> Self {
        Self {
            ok: true,
            error: None,
            consumed,
            commit,
            mode: mode_name(&snapshot.mode),
            composing: snapshot.composing,
            page: snapshot.page,
            visible_candidates: snapshot
                .visible_candidates
                .into_iter()
                .map(CandidateEnvelope::from)
                .collect(),
        }
    }

    fn error(message: &'static str) -> Self {
        Self {
            ok: false,
            error: Some(message),
            consumed: false,
            commit: None,
            mode: "English",
            composing: String::new(),
            page: 0,
            visible_candidates: Vec::new(),
        }
    }
}

#[derive(Serialize)]
struct CandidateEnvelope {
    text: String,
    annotation: String,
    source: &'static str,
    final_score: i32,
}

#[derive(Serialize)]
struct LearningEnvelope {
    ok: bool,
    error: Option<&'static str>,
    decision: &'static str,
    term: Option<String>,
}

#[derive(Serialize)]
struct ImportEnvelope {
    ok: bool,
    error: Option<&'static str>,
    imported: usize,
}

impl ImportEnvelope {
    fn ok(imported: usize) -> Self {
        Self {
            ok: true,
            error: None,
            imported,
        }
    }

    fn error(message: &'static str) -> Self {
        Self {
            ok: false,
            error: Some(message),
            imported: 0,
        }
    }
}

impl LearningEnvelope {
    fn from_outcome(outcome: inputia_core::LearningOutcome) -> Self {
        Self {
            ok: true,
            error: None,
            decision: privacy_decision_name(&outcome.decision),
            term: outcome.term,
        }
    }

    fn context_set() -> Self {
        Self {
            ok: true,
            error: None,
            decision: "context_set",
            term: None,
        }
    }

    fn error(message: &'static str) -> Self {
        Self {
            ok: false,
            error: Some(message),
            decision: "error",
            term: None,
        }
    }
}

#[derive(Serialize)]
struct HotwordsEnvelope {
    ok: bool,
    error: Option<&'static str>,
    hotwords: Vec<String>,
}

#[derive(Serialize)]
struct CandidateListEnvelope {
    ok: bool,
    error: Option<&'static str>,
    candidates: Vec<CandidateEnvelope>,
}

impl CandidateListEnvelope {
    fn ok(candidates: Vec<Candidate>) -> Self {
        Self {
            ok: true,
            error: None,
            candidates: candidates
                .into_iter()
                .map(CandidateEnvelope::from)
                .collect(),
        }
    }

    fn error(message: &'static str) -> Self {
        Self {
            ok: false,
            error: Some(message),
            candidates: Vec::new(),
        }
    }
}

impl HotwordsEnvelope {
    fn ok(hotwords: Vec<String>) -> Self {
        Self {
            ok: true,
            error: None,
            hotwords,
        }
    }

    fn error(message: &'static str) -> Self {
        Self {
            ok: false,
            error: Some(message),
            hotwords: Vec::new(),
        }
    }
}

impl From<Candidate> for CandidateEnvelope {
    fn from(candidate: Candidate) -> Self {
        let final_score = candidate.final_score();
        Self {
            text: candidate.text,
            annotation: candidate.annotation,
            source: match candidate.source {
                inputia_core::CandidateSource::Engine => "engine",
                inputia_core::CandidateSource::Memory => "memory",
                inputia_core::CandidateSource::Clipboard => "clipboard",
                inputia_core::CandidateSource::Voice => "voice",
                inputia_core::CandidateSource::EnglishCompletion => "english_completion",
            },
            final_score,
        }
    }
}

fn mode_name(mode: &InputMode) -> &'static str {
    match mode {
        InputMode::English => "English",
        InputMode::Chinese => "Chinese",
    }
}

fn privacy_decision_name(decision: &PrivacyDecision) -> &'static str {
    match decision {
        PrivacyDecision::Learn => "learn",
        PrivacyDecision::Excluded => "excluded",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{params, Connection};
    use serde_json::Value;

    static RIME_CAPI_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn capi_drives_core_with_rime_full_pinyin_when_available() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let user_data_dir =
            CString::new(shared_rime_user_data_dir().to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_luna_pinyin_simp(user_data_dir.as_ptr(), 2);
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let shift = handle_json(inputia_session_handle_special(session, KEY_SHIFT));
        assert_eq!(shift["mode"], "Chinese");

        let mut latest = shift;
        for ch in "zhongguo".chars() {
            latest = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        assert_eq!(latest["composing"], "zhongguo");
        assert_eq!(latest["visible_candidates"][0]["text"], "中国");

        let page_down = handle_json(inputia_session_handle_special(session, KEY_PAGE_DOWN));
        assert_eq!(page_down["page"], 1);
        assert_ne!(page_down["visible_candidates"][0]["text"], "中国");

        let page_up = handle_json(inputia_session_handle_special(session, KEY_PAGE_UP));
        assert_eq!(page_up["page"], 0);
        assert_eq!(page_up["visible_candidates"][0]["text"], "中国");

        let commit = handle_json(inputia_session_handle_char(session, ',' as u32));
        assert_eq!(commit["commit"], "中国,");
        assert_eq!(commit["composing"], "");

        inputia_session_free(session);
    }

    #[test]
    fn capi_enter_commits_raw_composition() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let user_data_dir =
            CString::new(shared_rime_user_data_dir().to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_luna_pinyin_simp(user_data_dir.as_ptr(), 5);
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        assert_eq!(
            handle_json(inputia_session_handle_special(session, KEY_SHIFT))["mode"],
            "Chinese"
        );
        for ch in "ni".chars() {
            let _ = handle_json(inputia_session_handle_char(session, ch as u32));
        }

        let commit = handle_json(inputia_session_handle_special(session, KEY_ENTER));
        assert_eq!(commit["commit"], "ni");
        assert_eq!(commit["composing"], "");
        assert!(commit["visible_candidates"].as_array().unwrap().is_empty());

        inputia_session_free(session);
    }

    #[test]
    fn capi_paginates_across_rime_candidate_pages() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let user_data_dir =
            CString::new(shared_rime_user_data_dir().to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_luna_pinyin_simp(user_data_dir.as_ptr(), 5);
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        assert_eq!(
            handle_json(inputia_session_handle_special(session, KEY_SHIFT))["mode"],
            "Chinese"
        );
        let mut latest = Value::Null;
        for ch in "ni".chars() {
            latest = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        assert_eq!(latest["visible_candidates"][0]["text"], "你");

        let page_down = handle_json(inputia_session_handle_special(session, KEY_PAGE_DOWN));
        assert_eq!(page_down["page"], 1);
        assert_ne!(page_down["visible_candidates"][0]["text"], "你");
        assert_eq!(page_down["visible_candidates"].as_array().unwrap().len(), 5);

        inputia_session_free(session);
    }

    #[test]
    fn capi_can_open_double_pinyin_schema_when_prepared() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let shared_data_dir = std::path::PathBuf::from("/tmp/inputia-rime-shared-double-pinyin");
        let user_data_dir = std::path::PathBuf::from("/tmp/inputia-rime-user-double-pinyin");
        if !shared_data_dir
            .join("double_pinyin_flypy.schema.yaml")
            .exists()
        {
            eprintln!(
                "skip: run spikes/inputia-rime/prepare-double-pinyin-data.sh double_pinyin_flypy first"
            );
            return;
        }

        let schema = CString::new("double_pinyin_flypy").unwrap();
        let dylib =
            CString::new("/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib")
                .unwrap();
        let shared = CString::new(shared_data_dir.to_string_lossy().as_bytes()).unwrap();
        let user = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_with_paths(
            schema.as_ptr(),
            dylib.as_ptr(),
            shared.as_ptr(),
            user.as_ptr(),
            2,
        );
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let shift = handle_json(inputia_session_handle_special(session, KEY_SHIFT));
        assert_eq!(shift["mode"], "Chinese");

        let mut latest = shift;
        for ch in "vsgo".chars() {
            latest = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        assert_eq!(latest["composing"], "vsgo");
        assert_eq!(latest["visible_candidates"][0]["text"], "中国");

        let commit = handle_json(inputia_session_handle_special(session, KEY_SPACE));
        assert_eq!(commit["commit"], "中国");

        inputia_session_free(session);
    }

    #[test]
    fn capi_memory_reranks_candidates_and_respects_sensitive_apps() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = shared_rime_user_data_dir();
        let memory_db = temp.path().join("inputia-memory.db");
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_luna_pinyin_simp_with_memory(
            user_data_dir.as_ptr(),
            memory_db.as_ptr(),
            5,
        );
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let source_app = CString::new("com.apple.TextEdit").unwrap();
        let remembered = CString::new("中国").unwrap();
        let learned = handle_json(inputia_session_learn(
            session,
            SOURCE_CLIPBOARD,
            remembered.as_ptr(),
            source_app.as_ptr(),
        ));
        assert_eq!(learned["decision"], "learn");
        assert_eq!(learned["term"], "中国");

        let sensitive_app = CString::new("com.1password.1password").unwrap();
        let sensitive_term = CString::new("密码 候选").unwrap();
        let excluded = handle_json(inputia_session_learn(
            session,
            SOURCE_CLIPBOARD,
            sensitive_term.as_ptr(),
            sensitive_app.as_ptr(),
        ));
        assert_eq!(excluded["decision"], "excluded");
        assert!(excluded["term"].is_null());

        let voice_term = CString::new("语音 热词").unwrap();
        let voice = handle_json(inputia_session_learn(
            session,
            SOURCE_VOICE,
            voice_term.as_ptr(),
            source_app.as_ptr(),
        ));
        assert_eq!(voice["decision"], "learn");

        let clipboard_candidates = handle_json(inputia_session_clipboard_candidates(session, 10));
        assert_eq!(clipboard_candidates["candidates"][0]["text"], "中国");
        assert_eq!(clipboard_candidates["candidates"][0]["source"], "clipboard");
        assert!(!clipboard_candidates["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["text"] == "语音 热词"));

        let shift = handle_json(inputia_session_handle_special(session, KEY_SHIFT));
        assert_eq!(shift["mode"], "Chinese");

        let mut latest = shift;
        for ch in "zhongguo".chars() {
            latest = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        assert_eq!(latest["visible_candidates"][0]["text"], "中国");
        assert_eq!(latest["visible_candidates"][0]["source"], "clipboard");

        let commit = handle_json(inputia_session_handle_special(session, KEY_SPACE));
        assert_eq!(commit["commit"], "中国");

        let hotwords = handle_json(inputia_session_voice_hotwords(session, 10));
        let hotword_values = hotwords["hotwords"].as_array().unwrap();
        assert!(hotword_values.iter().any(|value| value == "语音 热词"));
        assert!(hotword_values.iter().any(|value| value == "中国"));
        assert!(!hotword_values.iter().any(|value| value == "密码 候选"));

        inputia_session_free(session);
    }

    #[test]
    fn capi_memory_does_not_promote_single_character_over_long_double_pinyin_candidate() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let Some(shared_data_dir) = bundled_shared_data_dir() else {
            eprintln!("skip: Inputia bundled RimeData is not available");
            return;
        };
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let memory_db_path = temp.path().join("inputia-memory.db");
        let settings = InputiaSettings {
            schema_id: "double_pinyin".to_string(),
            rime_shared_data_dir: Some(shared_data_dir),
            rime_user_data_dir: Some(shared_rime_user_data_dir()),
            memory_db_path: Some(memory_db_path),
            spelling_correction_enabled: false,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_from_settings(settings_path.as_ptr());
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let source_app = CString::new("com.apple.TextEdit").unwrap();
        let single_char = CString::new("你").unwrap();
        for _ in 0..30 {
            let learned = handle_json(inputia_session_learn(
                session,
                SOURCE_TYPED,
                single_char.as_ptr(),
                source_app.as_ptr(),
            ));
            assert_eq!(learned["decision"], "learn");
        }

        assert_eq!(
            handle_json(inputia_session_set_input_mode(session, INPUT_MODE_CHINESE))["mode"],
            "Chinese"
        );
        let mut latest = serde_json::Value::Null;
        for ch in "nillem".chars() {
            latest = handle_json(inputia_session_handle_char(session, ch as u32));
        }

        assert_eq!(latest["visible_candidates"][0]["text"], "你来");
        assert_ne!(latest["visible_candidates"][0]["text"], "你");

        let selected_single = handle_json(inputia_session_handle_digit(session, 2));
        assert!(selected_single["commit"].is_null());
        assert_eq!(selected_single["composing"], "你laiem");
        assert_eq!(selected_single["visible_candidates"][0]["text"], "来");

        inputia_session_free(session);
    }

    #[test]
    fn capi_memory_keeps_segmented_phrase_ahead_of_single_character_across_schemas() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let Some(shared_data_dir) = bundled_shared_data_dir() else {
            eprintln!("skip: Inputia bundled RimeData is not available");
            return;
        };

        let cases = [
            "double_pinyin",
            "double_pinyin_flypy",
            "double_pinyin_sogou",
            "guobiao_bispell",
            "double_pinyin_mspy",
            "double_pinyin_abc",
            "double_pinyin_pyjj",
            "double_pinyin_st",
        ];
        let temp_root = tempfile::tempdir().unwrap();
        let source_app = CString::new("com.apple.TextEdit").unwrap();
        let single_char = CString::new("你").unwrap();

        for schema in cases {
            let case_root = temp_root.path().join(schema);
            let settings_path = case_root.join("settings.json");
            let rime_user_data_dir = case_root.join("rime-user");
            let memory_db_path = case_root.join("inputia-memory.db");
            std::fs::create_dir_all(&rime_user_data_dir).unwrap();
            let settings = InputiaSettings {
                schema_id: schema.to_string(),
                rime_shared_data_dir: Some(shared_data_dir.clone()),
                rime_user_data_dir: Some(rime_user_data_dir),
                memory_db_path: Some(memory_db_path),
                spelling_correction_enabled: false,
                candidate_page_size: 9,
                ..InputiaSettings::default()
            };
            settings.save(&settings_path).unwrap();
            let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
            let session = inputia_session_new_from_settings(settings_path.as_ptr());
            if session.is_null() {
                eprintln!("skip: Squirrel librime runtime is not available");
                return;
            }

            for _ in 0..30 {
                let learned = handle_json(inputia_session_learn(
                    session,
                    SOURCE_TYPED,
                    single_char.as_ptr(),
                    source_app.as_ptr(),
                ));
                assert_eq!(learned["decision"], "learn", "{schema}");
            }

            assert_eq!(
                handle_json(inputia_session_set_input_mode(session, INPUT_MODE_CHINESE))["mode"],
                "Chinese",
                "{schema}"
            );
            let mut latest = Value::Null;
            for ch in "nillem".chars() {
                latest = handle_json(inputia_session_handle_char(session, ch as u32));
            }

            let candidates = latest["visible_candidates"].as_array().unwrap();
            let first_text = candidates[0]["text"].as_str().unwrap();
            assert!(
                first_text.chars().count() > 1,
                "{schema} should keep a phrase ahead of the learned single-character candidate"
            );
            let single_index = candidates
                .iter()
                .position(|candidate| candidate["text"] == "你")
                .unwrap_or(usize::MAX);
            if single_index == usize::MAX {
                inputia_session_free(session);
                continue;
            }
            assert!(
                single_index > 0,
                "{schema} should not rank the learned single-character candidate first"
            );

            let selected_single = handle_json(inputia_session_handle_digit(
                session,
                (single_index + 1) as u8,
            ));
            assert!(
                selected_single["commit"].is_null(),
                "{schema} should keep partial single-character selection in composition"
            );
            assert!(
                !selected_single["composing"]
                    .as_str()
                    .unwrap_or_default()
                    .is_empty(),
                "{schema} should keep remaining composition after single-character selection"
            );
            assert!(
                !selected_single["visible_candidates"]
                    .as_array()
                    .unwrap()
                    .is_empty(),
                "{schema} should keep candidates after single-character selection"
            );

            inputia_session_free(session);
        }
    }

    #[test]
    fn capi_returns_english_completion_candidates_from_typed_memory() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = shared_rime_user_data_dir();
        let memory_db = temp.path().join("inputia-memory.db");
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_luna_pinyin_simp_with_memory(
            user_data_dir.as_ptr(),
            memory_db.as_ptr(),
            5,
        );
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let source_app = CString::new("com.apple.TextEdit").unwrap();
        let inputia = CString::new("Inputia").unwrap();
        let input_layer = CString::new("input-layer").unwrap();
        assert_eq!(
            handle_json(inputia_session_learn(
                session,
                SOURCE_TYPED,
                inputia.as_ptr(),
                source_app.as_ptr(),
            ))["decision"],
            "learn"
        );
        assert_eq!(
            handle_json(inputia_session_learn(
                session,
                SOURCE_TYPED,
                inputia.as_ptr(),
                source_app.as_ptr(),
            ))["decision"],
            "learn"
        );
        assert_eq!(
            handle_json(inputia_session_learn(
                session,
                SOURCE_CLIPBOARD,
                input_layer.as_ptr(),
                source_app.as_ptr(),
            ))["decision"],
            "learn"
        );

        let prefix = CString::new("in").unwrap();
        let completions = handle_json(inputia_session_completion_candidates(
            session,
            prefix.as_ptr(),
            5,
        ));

        assert_eq!(completions["ok"], true);
        assert_eq!(completions["candidates"][0]["text"], "Inputia");
        assert_eq!(completions["candidates"][0]["source"], "english_completion");
        assert!(completions["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["text"] == "input-layer"));

        inputia_session_free(session);
    }

    #[test]
    fn capi_imports_handy_history_into_voice_hotwords_and_ranking() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = shared_rime_user_data_dir();
        let memory_db = temp.path().join("inputia-memory.db");
        let history_db = temp.path().join("history.db");
        create_handy_history_db(&history_db);
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let history_db = CString::new(history_db.to_string_lossy().as_bytes()).unwrap();
        let bundle_id = CString::new("com.pais.handy").unwrap();
        let session = inputia_session_new_luna_pinyin_simp_with_memory(
            user_data_dir.as_ptr(),
            memory_db.as_ptr(),
            5,
        );
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let imported = handle_json(inputia_session_import_handy_history(
            session,
            history_db.as_ptr(),
            bundle_id.as_ptr(),
            10,
        ));
        assert_eq!(imported["ok"], true);
        assert_eq!(imported["imported"], 2);

        let hotwords = handle_json(inputia_session_voice_hotwords(session, 10));
        let hotword_values = hotwords["hotwords"].as_array().unwrap();
        assert!(hotword_values.iter().any(|value| value == "中国"));
        assert!(hotword_values.iter().any(|value| value == "语音 热词"));

        assert_eq!(
            handle_json(inputia_session_handle_special(session, KEY_SHIFT))["mode"],
            "Chinese"
        );
        let mut latest = Value::Null;
        for ch in "zhongguo".chars() {
            latest = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        assert_eq!(latest["visible_candidates"][0]["text"], "中国");
        assert_eq!(latest["visible_candidates"][0]["source"], "voice");

        inputia_session_free(session);
    }

    #[test]
    fn capi_imports_handy_clipboard_and_skips_sensitive_source_apps() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = shared_rime_user_data_dir();
        let memory_db = temp.path().join("inputia-memory.db");
        let clipboard_db = temp.path().join("clipboard.db");
        create_handy_clipboard_db(&clipboard_db);
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let clipboard_db = CString::new(clipboard_db.to_string_lossy().as_bytes()).unwrap();
        let bundle_id = CString::new("com.pais.handy").unwrap();
        let session = inputia_session_new_luna_pinyin_simp_with_memory(
            user_data_dir.as_ptr(),
            memory_db.as_ptr(),
            5,
        );
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let imported = handle_json(inputia_session_import_handy_clipboard(
            session,
            clipboard_db.as_ptr(),
            bundle_id.as_ptr(),
            10,
        ));
        assert_eq!(imported["ok"], true);
        assert_eq!(imported["imported"], 1);

        let clipboard_candidates = handle_json(inputia_session_clipboard_candidates(session, 10));
        assert_eq!(clipboard_candidates["candidates"][0]["text"], "中国");
        assert_eq!(clipboard_candidates["candidates"][0]["source"], "clipboard");
        assert!(!clipboard_candidates["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["text"] == "密码 候选"));

        assert_eq!(
            handle_json(inputia_session_handle_special(session, KEY_SHIFT))["mode"],
            "Chinese"
        );
        let mut latest = Value::Null;
        for ch in "zhongguo".chars() {
            latest = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        assert_eq!(latest["visible_candidates"][0]["text"], "中国");
        assert_eq!(latest["visible_candidates"][0]["source"], "clipboard");

        inputia_session_free(session);
    }

    #[test]
    fn capi_typed_commits_respect_current_app_context() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = shared_rime_user_data_dir();
        let memory_db = temp.path().join("inputia-memory.db");
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_luna_pinyin_simp_with_memory(
            user_data_dir.as_ptr(),
            memory_db.as_ptr(),
            5,
        );
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let sensitive_app = CString::new("com.1password.1password").unwrap();
        let context = handle_json(inputia_session_set_app_context(
            session,
            sensitive_app.as_ptr(),
        ));
        assert_eq!(context["decision"], "context_set");

        let shift = handle_json(inputia_session_handle_special(session, KEY_SHIFT));
        assert_eq!(shift["mode"], "Chinese");

        for ch in "zhongguo".chars() {
            let _ = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        let commit = handle_json(inputia_session_handle_special(session, KEY_SPACE));
        assert_eq!(commit["commit"], "中国");

        let hotwords = handle_json(inputia_session_voice_hotwords(session, 10));
        let hotword_values = hotwords["hotwords"].as_array().unwrap();
        assert!(!hotword_values.iter().any(|value| value == "中国"));

        inputia_session_free(session);
    }

    #[test]
    fn capi_window_contexts_can_block_learning() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = shared_rime_user_data_dir();
        let memory_db = temp.path().join("inputia-memory.db");
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_luna_pinyin_simp_with_memory(
            user_data_dir.as_ptr(),
            memory_db.as_ptr(),
            5,
        );
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let bundle_id = CString::new("com.apple.Safari").unwrap();
        let window_title = CString::new("Private Browsing - Bank Login").unwrap();
        let context = handle_json(inputia_session_set_app_context_with_window(
            session,
            bundle_id.as_ptr(),
            window_title.as_ptr(),
        ));
        assert_eq!(context["decision"], "context_set");

        let secret = CString::new("secret phrase").unwrap();
        let learned = handle_json(inputia_session_learn(
            session,
            SOURCE_TYPED,
            secret.as_ptr(),
            bundle_id.as_ptr(),
        ));
        assert_eq!(learned["decision"], "excluded");

        assert_eq!(
            handle_json(inputia_session_handle_special(session, KEY_SHIFT))["mode"],
            "Chinese"
        );
        for ch in "zhongguo".chars() {
            let _ = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        let commit = handle_json(inputia_session_handle_special(session, KEY_SPACE));
        assert_eq!(commit["commit"], "中国");

        let hotwords = handle_json(inputia_session_voice_hotwords(session, 10));
        let hotword_values = hotwords["hotwords"].as_array().unwrap();
        assert!(!hotword_values.iter().any(|value| value == "中国"));
        assert!(!hotword_values.iter().any(|value| value == "secret phrase"));

        inputia_session_free(session);
    }

    #[test]
    fn capi_loads_shift_setting_from_settings_file() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            rime_user_data_dir: Some(shared_rime_user_data_dir()),
            memory_enabled: false,
            shift_toggle_enabled: false,
            input_mode_toggle_shortcut: inputia_settings::InputModeToggleShortcut::None,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_from_settings(settings_path.as_ptr());
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let shift = handle_json(inputia_session_handle_special(session, KEY_SHIFT));

        assert_eq!(shift["consumed"], false);
        assert_eq!(shift["mode"], "English");

        inputia_session_free(session);
    }

    #[test]
    fn capi_explicit_input_mode_toggle_supports_remapped_shortcut() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            rime_user_data_dir: Some(shared_rime_user_data_dir()),
            memory_enabled: false,
            shift_toggle_enabled: false,
            input_mode_toggle_shortcut: inputia_settings::InputModeToggleShortcut::ControlSpace,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_from_settings(settings_path.as_ptr());
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let ignored_shift = handle_json(inputia_session_handle_special(session, KEY_SHIFT));
        assert_eq!(ignored_shift["mode"], "English");
        assert_eq!(ignored_shift["consumed"], false);

        let remapped_toggle = handle_json(inputia_session_handle_special(
            session,
            KEY_TOGGLE_INPUT_MODE,
        ));
        assert_eq!(remapped_toggle["mode"], "Chinese");
        assert_eq!(remapped_toggle["consumed"], true);

        inputia_session_free(session);
    }

    #[test]
    fn capi_sets_input_mode_explicitly() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let user_data_dir =
            CString::new(shared_rime_user_data_dir().to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_luna_pinyin_simp(user_data_dir.as_ptr(), 5);
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let set_chinese = handle_json(inputia_session_set_input_mode(session, INPUT_MODE_CHINESE));
        assert_eq!(set_chinese["mode"], "Chinese");
        assert_eq!(set_chinese["consumed"], false);

        let z = handle_json(inputia_session_handle_char(session, 'z' as u32));
        assert_eq!(z["mode"], "Chinese");
        assert_eq!(z["composing"], "z");

        let set_english = handle_json(inputia_session_set_input_mode(session, INPUT_MODE_ENGLISH));
        assert_eq!(set_english["mode"], "English");

        let direct = handle_json(inputia_session_handle_char(session, 'x' as u32));
        assert_eq!(direct["mode"], "English");
        assert_eq!(direct["commit"], "x");

        inputia_session_free(session);
    }

    #[test]
    fn capi_loads_candidate_count_and_punctuation_from_settings_file() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            rime_user_data_dir: Some(shared_rime_user_data_dir()),
            memory_enabled: false,
            candidate_page_size: 2,
            punctuation_preference: inputia_settings::PunctuationPreference::FollowInputMode,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_from_settings(settings_path.as_ptr());
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let shift = handle_json(inputia_session_handle_special(session, KEY_SHIFT));
        assert_eq!(shift["mode"], "Chinese");

        let mut latest = shift;
        for ch in "zhongguo".chars() {
            latest = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        assert_eq!(latest["visible_candidates"].as_array().unwrap().len(), 2);

        let comma = handle_json(inputia_session_handle_char(session, ',' as u32));
        assert_eq!(comma["commit"], "中国，");
        assert_eq!(comma["composing"], "");

        inputia_session_free(session);
    }

    #[test]
    fn capi_backfills_legacy_settings_paths_and_shared_data_fallback() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        if bundled_shared_data_dir().is_none() {
            eprintln!("skip: Inputia bundled RimeData is not available");
            return;
        }

        let legacy_root = std::env::temp_dir().join(format!(
            "inputia-capi-legacy-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let settings_path = legacy_root.join("settings.json");
        std::fs::create_dir_all(&legacy_root).unwrap();
        std::fs::write(
            &settings_path,
            format!(
                r#"{{
                  "schema_id": "double_pinyin",
                  "candidate_page_size": 7,
                  "memory_enabled": false,
                  "rime_shared_data_dir": "{}"
                }}"#,
                legacy_root.join("missing-rime-data").display()
            ),
        )
        .unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_from_settings(settings_path.as_ptr());
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        assert_eq!(
            handle_json(inputia_session_set_input_mode(session, INPUT_MODE_CHINESE))["mode"],
            "Chinese"
        );

        let mut latest = serde_json::Value::Null;
        for ch in "nillem".chars() {
            latest = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        assert_eq!(latest["visible_candidates"][0]["text"], "你来");
        assert_eq!(latest["visible_candidates"].as_array().unwrap().len(), 7);

        inputia_session_free(session);
    }

    #[test]
    fn capi_settings_schemas_commit_zhongguo_when_available() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let Some(shared_data_dir) = bundled_shared_data_dir() else {
            eprintln!("skip: Inputia bundled RimeData is not available");
            return;
        };

        let cases = [
            SettingsSchemaSmokeCase {
                schema: "luna_pinyin_simp",
                keys: "zhongguo",
            },
            SettingsSchemaSmokeCase {
                schema: "double_pinyin",
                keys: "vsgo",
            },
            SettingsSchemaSmokeCase {
                schema: "double_pinyin_flypy",
                keys: "vsgo",
            },
            SettingsSchemaSmokeCase {
                schema: "double_pinyin_sogou",
                keys: "vsgo",
            },
            SettingsSchemaSmokeCase {
                schema: "guobiao_bispell",
                keys: "vsgo",
            },
            SettingsSchemaSmokeCase {
                schema: "double_pinyin_mspy",
                keys: "vsgo",
            },
            SettingsSchemaSmokeCase {
                schema: "double_pinyin_abc",
                keys: "asgo",
            },
            SettingsSchemaSmokeCase {
                schema: "double_pinyin_pyjj",
                keys: "vygo",
            },
            SettingsSchemaSmokeCase {
                schema: "double_pinyin_st",
                keys: "aygo",
            },
        ];

        let temp_root = tempfile::tempdir().unwrap();
        for case in cases {
            let case_root = temp_root.path().join(case.schema);
            let settings_path = case_root.join("settings.json");
            let rime_user_data_dir = case_root.join("rime-user");
            std::fs::create_dir_all(&rime_user_data_dir).unwrap();
            let settings = InputiaSettings {
                schema_id: case.schema.to_string(),
                rime_shared_data_dir: Some(shared_data_dir.clone()),
                rime_user_data_dir: Some(rime_user_data_dir),
                memory_enabled: false,
                candidate_page_size: 7,
                ..InputiaSettings::default()
            };
            settings.save(&settings_path).unwrap();
            let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
            let session = inputia_session_new_from_settings(settings_path.as_ptr());
            if session.is_null() {
                eprintln!("skip: Squirrel librime runtime is not available");
                return;
            }

            assert_eq!(
                handle_json(inputia_session_set_input_mode(session, INPUT_MODE_CHINESE))["mode"],
                "Chinese"
            );
            let mut latest = Value::Null;
            for ch in case.keys.chars() {
                latest = handle_json(inputia_session_handle_char(session, ch as u32));
            }
            assert_eq!(latest["composing"], case.keys, "{}", case.schema);
            assert_eq!(
                latest["visible_candidates"][0]["text"], "中国",
                "{} should rank 中国 first for {} through settings",
                case.schema, case.keys
            );
            assert_eq!(
                latest["visible_candidates"].as_array().unwrap().len(),
                7,
                "{} should expose seven settings-page candidates",
                case.schema
            );

            let commit = handle_json(inputia_session_handle_special(session, KEY_SPACE));
            assert_eq!(
                commit["commit"], "中国",
                "{} should commit 中国 for {} through settings",
                case.schema, case.keys
            );
            assert_eq!(commit["composing"], "");

            inputia_session_free(session);
        }
    }

    #[test]
    fn capi_settings_guobiao_bispell_commits_standard_maile_sequence() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let Some(shared_data_dir) = bundled_shared_data_dir() else {
            eprintln!("skip: Inputia bundled RimeData is not available");
            return;
        };

        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let rime_user_data_dir = temp.path().join("rime-user");
        std::fs::create_dir_all(&rime_user_data_dir).unwrap();
        let settings = InputiaSettings {
            schema_id: "guobiao_bispell".to_string(),
            rime_shared_data_dir: Some(shared_data_dir),
            rime_user_data_dir: Some(rime_user_data_dir),
            memory_enabled: false,
            candidate_page_size: 7,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_from_settings(settings_path.as_ptr());
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        assert_eq!(
            handle_json(inputia_session_set_input_mode(session, INPUT_MODE_CHINESE))["mode"],
            "Chinese"
        );

        let mut latest = Value::Null;
        for ch in "mkle".chars() {
            latest = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        assert_eq!(latest["composing"], "mkle");
        assert_eq!(latest["visible_candidates"][0]["text"], "买了");
        assert_ne!(latest["visible_candidates"][0]["text"], "mkle");

        let commit = handle_json(inputia_session_handle_special(session, KEY_SPACE));
        assert_eq!(commit["commit"], "买了");
        assert_eq!(commit["composing"], "");

        inputia_session_free(session);
    }

    #[test]
    fn capi_loads_full_width_setting_from_settings_file() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            rime_user_data_dir: Some(shared_rime_user_data_dir()),
            memory_enabled: false,
            character_width_preference: inputia_settings::CharacterWidthPreference::FullWidth,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_from_settings(settings_path.as_ptr());
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let direct = handle_json(inputia_session_handle_char(session, 'A' as u32));
        assert_eq!(direct["mode"], "English");
        assert_eq!(direct["commit"], "Ａ");

        assert_eq!(
            handle_json(inputia_session_set_input_mode(session, INPUT_MODE_CHINESE))["mode"],
            "Chinese"
        );
        for ch in "ni".chars() {
            let _ = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        let raw = handle_json(inputia_session_handle_special(session, KEY_ENTER));
        assert_eq!(raw["commit"], "ｎｉ");

        inputia_session_free(session);
    }

    #[test]
    fn capi_loads_spelling_correction_setting_from_settings_file() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            rime_user_data_dir: Some(shared_rime_user_data_dir()),
            memory_enabled: false,
            spelling_correction_enabled: true,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_from_settings(settings_path.as_ptr());
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        assert_eq!(
            handle_json(inputia_session_set_input_mode(session, INPUT_MODE_CHINESE))["mode"],
            "Chinese"
        );
        let mut latest = Value::Null;
        for ch in "dagn".chars() {
            latest = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        assert_eq!(latest["visible_candidates"][0]["text"], "当");

        inputia_session_free(session);
    }

    #[test]
    fn spelling_correction_is_effective_only_for_full_pinyin_schemas() {
        assert!(effective_spelling_correction("luna_pinyin_simp", true));
        assert!(effective_spelling_correction("luna_pinyin", true));
        assert!(!effective_spelling_correction("luna_pinyin_simp", false));
        assert!(!effective_spelling_correction("double_pinyin", true));
        assert!(!effective_spelling_correction("double_pinyin_sogou", true));
        assert!(!effective_spelling_correction("guobiao_bispell", true));
    }

    #[test]
    fn chinese_script_selects_schema_and_rime_option() {
        assert_eq!(
            effective_rime_script_config(
                "luna_pinyin_simp",
                &inputia_settings::ChineseScript::Traditional
            ),
            (
                "luna_pinyin".to_string(),
                vec![
                    ("simplification".to_string(), false),
                    ("zh_hans".to_string(), false),
                    ("zh_hant".to_string(), true)
                ]
            )
        );
        assert_eq!(
            effective_rime_script_config(
                "luna_pinyin_simp",
                &inputia_settings::ChineseScript::Simplified
            ),
            (
                "luna_pinyin_simp".to_string(),
                vec![("simplification".to_string(), true)]
            )
        );
        assert_eq!(
            effective_rime_script_config(
                "luna_pinyin",
                &inputia_settings::ChineseScript::Simplified
            ),
            (
                "luna_pinyin".to_string(),
                vec![
                    ("simplification".to_string(), true),
                    ("zh_hant".to_string(), false),
                    ("zh_hans".to_string(), true)
                ]
            )
        );
        assert_eq!(
            effective_rime_script_config(
                "luna_pinyin",
                &inputia_settings::ChineseScript::Traditional
            ),
            (
                "luna_pinyin".to_string(),
                vec![
                    ("simplification".to_string(), false),
                    ("zh_hans".to_string(), false),
                    ("zh_hant".to_string(), true)
                ]
            )
        );
        assert_eq!(
            effective_rime_script_config(
                "double_pinyin_flypy",
                &inputia_settings::ChineseScript::Traditional
            ),
            (
                "double_pinyin_flypy".to_string(),
                vec![("simplification".to_string(), false)]
            )
        );
        assert_eq!(
            effective_rime_script_config(
                "guobiao_bispell",
                &inputia_settings::ChineseScript::Traditional
            ),
            (
                "guobiao_bispell".to_string(),
                vec![
                    ("simplification".to_string(), false),
                    ("trad_tw".to_string(), true)
                ]
            )
        );
    }

    #[test]
    fn capi_traditional_script_commits_traditional_chinese() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            rime_user_data_dir: Some(shared_rime_user_data_dir()),
            memory_enabled: false,
            chinese_script: inputia_settings::ChineseScript::Traditional,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_from_settings(settings_path.as_ptr());
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        assert_eq!(
            handle_json(inputia_session_set_input_mode(session, INPUT_MODE_CHINESE))["mode"],
            "Chinese"
        );
        for ch in "zhongguo".chars() {
            let _ = handle_json(inputia_session_handle_char(session, ch as u32));
        }
        let commit = handle_json(inputia_session_handle_special(session, KEY_SPACE));
        assert_eq!(commit["commit"], "中國");

        inputia_session_free(session);
    }

    #[test]
    fn capi_new_settings_session_survives_previous_session_free() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let Some(shared_data_dir) = bundled_shared_data_dir() else {
            eprintln!("skip: Inputia bundled RimeData is not available");
            return;
        };

        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            schema_id: "double_pinyin".to_string(),
            rime_shared_data_dir: Some(shared_data_dir),
            rime_user_data_dir: Some(shared_rime_user_data_dir()),
            memory_enabled: false,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let previous_session = inputia_session_new_from_settings(settings_path.as_ptr());
        if previous_session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }
        let next_session = inputia_session_new_from_settings(settings_path.as_ptr());
        assert!(!next_session.is_null());

        inputia_session_free(previous_session);

        assert_eq!(
            handle_json(inputia_session_set_input_mode(
                next_session,
                INPUT_MODE_CHINESE
            ))["mode"],
            "Chinese"
        );
        let mut latest = Value::Null;
        for ch in "mlle".chars() {
            latest = handle_json(inputia_session_handle_char(next_session, ch as u32));
        }
        assert_eq!(latest["composing"], "mlle");
        assert_eq!(latest["visible_candidates"][0]["text"], "买了");

        inputia_session_free(next_session);
    }

    #[test]
    fn capi_toggles_punctuation_and_character_width_runtime() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let user_data_dir =
            CString::new(shared_rime_user_data_dir().to_string_lossy().as_bytes()).unwrap();
        let session = inputia_session_new_luna_pinyin_simp(user_data_dir.as_ptr(), 5);
        if session.is_null() {
            eprintln!("skip: Squirrel librime runtime is not available");
            return;
        }

        let half_width = handle_json(inputia_session_handle_char(session, 'A' as u32));
        assert_eq!(half_width["commit"], "A");

        let toggle_width = handle_json(inputia_session_handle_special(
            session,
            KEY_TOGGLE_CHARACTER_WIDTH,
        ));
        assert_eq!(toggle_width["mode"], "English");
        assert_eq!(toggle_width["consumed"], true);

        let full_width = handle_json(inputia_session_handle_char(session, 'A' as u32));
        assert_eq!(full_width["commit"], "Ａ");

        assert_eq!(
            handle_json(inputia_session_set_input_mode(session, INPUT_MODE_CHINESE))["mode"],
            "Chinese"
        );
        let english_punctuation = handle_json(inputia_session_handle_char(session, ',' as u32));
        assert_eq!(english_punctuation["commit"], ",");

        let toggle_punctuation = handle_json(inputia_session_handle_special(
            session,
            KEY_TOGGLE_PUNCTUATION,
        ));
        assert_eq!(toggle_punctuation["mode"], "Chinese");
        assert_eq!(toggle_punctuation["consumed"], true);

        let chinese_punctuation = handle_json(inputia_session_handle_char(session, ',' as u32));
        assert_eq!(chinese_punctuation["commit"], "，");

        inputia_session_free(session);
    }

    fn handle_json(raw: *mut c_char) -> Value {
        assert!(!raw.is_null());
        let text = unsafe { CStr::from_ptr(raw).to_string_lossy().into_owned() };
        inputia_string_free(raw);
        serde_json::from_str(&text).unwrap()
    }

    #[derive(Clone, Copy)]
    struct SettingsSchemaSmokeCase {
        schema: &'static str,
        keys: &'static str,
    }

    fn create_handy_history_db(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE transcription_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                file_name TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                saved BOOLEAN NOT NULL DEFAULT 0,
                title TEXT NOT NULL,
                transcription_text TEXT NOT NULL,
                post_processed_text TEXT,
                post_process_prompt TEXT,
                post_process_requested BOOLEAN NOT NULL DEFAULT 0
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcription_history (
                file_name, timestamp, saved, title, transcription_text, post_processed_text
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "voice-1.wav",
                1,
                false,
                "Voice 1",
                "中国",
                Option::<String>::None
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcription_history (
                file_name, timestamp, saved, title, transcription_text, post_processed_text
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params!["voice-2.wav", 2, false, "Voice 2", "raw draft", "语音 热词"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcription_history (
                file_name, timestamp, saved, title, transcription_text, post_processed_text
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                "voice-3.wav",
                3,
                false,
                "Voice 3",
                "   ",
                Option::<String>::None
            ],
        )
        .unwrap();
    }

    fn create_handy_clipboard_db(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE clipboard_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                content_type TEXT NOT NULL,
                content_preview TEXT NOT NULL,
                content_hash TEXT NOT NULL UNIQUE,
                full_text TEXT,
                image_path TEXT,
                source_app TEXT,
                is_favorite BOOLEAN NOT NULL DEFAULT 0,
                is_pinned BOOLEAN NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                size_bytes INTEGER NOT NULL,
                title TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clipboard_history (
                content_type, content_preview, content_hash, full_text, source_app, created_at, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                "text",
                "中国",
                "hash-1",
                "中国",
                "com.apple.TextEdit",
                1,
                6
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clipboard_history (
                content_type, content_preview, content_hash, full_text, source_app, created_at, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                "text",
                "密码 候选",
                "hash-2",
                "密码 候选",
                "com.1password.1password",
                2,
                12
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clipboard_history (
                content_type, content_preview, content_hash, full_text, image_path, created_at, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                "image",
                "[image]",
                "hash-3",
                Option::<String>::None,
                "/tmp/ignored.png",
                3,
                1024
            ],
        )
        .unwrap();
    }

    fn bundled_shared_data_dir() -> Option<std::path::PathBuf> {
        if let Ok(path) = std::env::var("INPUTIA_RIME_SHARED_DATA_DIR") {
            let path = std::path::PathBuf::from(path);
            if path.exists() {
                return Some(path);
            }
        }

        for path in [
            std::path::PathBuf::from(
                "/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/RimeData",
            ),
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../macos/InputiaInputMethod/build/RimeData"),
        ] {
            if path.exists() {
                return Some(path);
            }
        }

        None
    }

    fn shared_rime_user_data_dir() -> std::path::PathBuf {
        let path = std::env::temp_dir().join("inputia-capi-rime-user");
        std::fs::create_dir_all(&path).expect("rime user data dir should be writable");
        path
    }
}
