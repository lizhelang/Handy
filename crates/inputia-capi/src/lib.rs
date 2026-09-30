//! C ABI 的内存安全由调用方提供，null 检查不能证明任意非空指针有效。
//!
//! 非空输入字符串须在调用期间指向可读、NUL 结尾且不被并发修改的缓冲区；
//! 函数会复制需要保留的字符串。session 必须来自本库成功的构造函数，仍存活且独占使用。
//! 同一活跃 Rime 运行时的 session 操作/创建/释放须在单一所有者线程串行执行。
//! 返回的 JSON 字符串独立归调用方持有，只能交回本库 inputia_string_free 一次。
//! Rust 的 unsafe 声明不改变 C/Swift ABI；各函数保留现有 null 返回或错误 JSON 行为。
//!
//! 即使具体调用使用允许的 null，Rust 调用点也必须明确承担 FFI 合同：
//! ```compile_fail
//! inputia_capi::inputia_session_new_luna_pinyin_simp(std::ptr::null(), 5);
//! ```
//! ```compile_fail
//! inputia_capi::inputia_session_handle_char(std::ptr::null_mut(), 'a' as u32);
//! ```

#![deny(unsafe_op_in_unsafe_fn)]

pub mod installation;
pub mod maintenance;

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_int};
use std::ptr::null_mut;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use inputia_core::{
    AppContext, AppPolicy, Candidate, CandidateSelection, CharacterWidthPreference, ChineseEngine,
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
    context_verified: Arc<AtomicBool>,
}

struct SessionOptions {
    rime: RimeEngineConfig,
    core: CoreSettings,
    memory_db_path: Option<String>,
    policy: AppPolicy,
}

struct RankedRimeEngine {
    context_verified: Arc<AtomicBool>,
    rime: RimeEngine,
    memory: Option<Arc<Mutex<SqliteMemory>>>,
}

impl ChineseEngine for RankedRimeEngine {
    fn undo_recent_learning(&self) -> bool {
        self.rime.undo_recent_learning().is_ok()
    }
    fn candidates(&self, composing: &str) -> Vec<Candidate> {
        self.candidates_up_to(composing, 10)
    }

    fn candidates_up_to(&self, composing: &str, minimum_count: usize) -> Vec<Candidate> {
        let candidates = self.rime.candidates_up_to(composing, minimum_count);
        if !self.context_verified.load(Ordering::Relaxed) {
            return candidates;
        }
        let Some(memory) = &self.memory else {
            return candidates;
        };

        let Ok(memory) = memory.lock() else {
            return candidates;
        };
        memory
            .rank_candidates_for_composing(composing, candidates.clone())
            .unwrap_or(candidates)
    }

    fn candidate_consumed_len(&self, composing: &str, candidate: &Candidate) -> Option<usize> {
        self.rime.candidate_consumed_len(composing, candidate)
    }

    fn select_candidate(
        &self,
        composing: &str,
        page: usize,
        page_index: usize,
        candidate: &Candidate,
    ) -> Option<CandidateSelection> {
        let mut selection = self
            .rime
            .select_candidate(composing, page, page_index, candidate)
            .ok()?;
        if !self.context_verified.load(Ordering::Relaxed) {
            return Some(selection);
        }
        let Some(memory) = &self.memory else {
            return Some(selection);
        };
        let Ok(memory) = memory.lock() else {
            return Some(selection);
        };
        selection.candidates = memory
            .rank_candidates_for_composing(&selection.composing, selection.candidates.clone())
            .unwrap_or(selection.candidates);
        Some(selection)
    }
}

/// 使用默认全拼配置创建 session。
///
/// # Safety
/// 非空 user_data_dir 必须满足模块的 C 字符串合同；创建须与所有活跃 Rime 调用串行。
/// 返回的非空 session 由调用方独占持有，并最终交给 inputia_session_free 一次。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_new_luna_pinyin_simp(
    user_data_dir: *const c_char,
    candidate_page_size: usize,
) -> *mut InputiaSession {
    new_session("luna_pinyin_simp", user_data_dir, candidate_page_size, None)
}

/// 创建带本地记忆的默认全拼 session。
///
/// # Safety
/// 两个非空路径指针须在调用期间保持有效的可读 NUL 结尾字符串；遵守单一所有者线程合同。
/// 返回 session 的所有权和释放要求同 inputia_session_new_luna_pinyin_simp。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_new_luna_pinyin_simp_with_memory(
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

/// 使用指定 schema 创建 session。
///
/// # Safety
/// 非空 schema_id 和 user_data_dir 须满足 C 字符串合同；创建须与其他 Rime 调用串行。
/// 返回的非空 session 必须保持独占并且只释放一次。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_new_with_schema(
    schema_id: *const c_char,
    user_data_dir: *const c_char,
    candidate_page_size: usize,
) -> *mut InputiaSession {
    let Some(schema_id) = (unsafe { optional_c_string(schema_id) }) else {
        return null_mut();
    };
    new_session(&schema_id, user_data_dir, candidate_page_size, None)
}

/// 使用显式引擎和数据路径创建 session。
///
/// # Safety
/// 所有非空字符串参数须在调用期间可读、NUL 结尾且不被修改；创建须与其他 Rime 调用串行。
/// 静态 feature 下仍要求 dylib_path 指针有效，虽然该配置路径不会被加载。
/// 返回的非空 session 必须保持独占并且只释放一次。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_new_with_paths(
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

/// 从设置文件创建 session。
///
/// # Safety
/// 非空 settings_path 须满足 C 字符串合同；创建须与其他 Rime 调用串行。
/// 返回的非空 session 由调用方独占，最终只释放一次。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_new_from_settings(
    settings_path: *const c_char,
) -> *mut InputiaSession {
    if inputia_settings::maintenance::ensure_current_normal_start().is_err() {
        return null_mut();
    }
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

/// 保留设置中的输入行为，但不打开记忆库。
///
/// # Safety
/// 非空 settings_path 须满足 C 字符串合同；创建须与其他 Rime 调用串行。
/// 返回的非空 session 由调用方独占，最终只释放一次。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_new_from_settings_without_memory(
    settings_path: *const c_char,
) -> *mut InputiaSession {
    if inputia_settings::maintenance::ensure_current_normal_start().is_err() {
        return null_mut();
    }
    let Some(settings_path) = (unsafe { optional_c_string(settings_path) }) else {
        return null_mut();
    };
    let Ok(settings) = InputiaSettings::load_or_create(&settings_path) else {
        return null_mut();
    };
    let Some(mut options) = session_options_from_settings(settings) else {
        return null_mut();
    };
    options.memory_db_path = None;
    new_session_with_options(options)
}

/// 释放 session；null 是无操作。
///
/// # Safety
/// 非空 session 必须是本库返回的原始、对齐且尚未释放的指针；不能是副本分配或内部地址。
/// 调用前须结束所有借用/操作，不得并发使用或释放；同一指针只释放一次。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_free(session: *mut InputiaSession) {
    if !session.is_null() {
        unsafe { drop(Box::from_raw(session)) };
    }
}

/// 处理一个 Unicode scalar，返回调用方所有的 JSON。
///
/// # Safety
/// 非空 session 必须仍存活且来自本库；在其所有者线程独占串行调用，不与任何 Rime 操作并发。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_handle_char(
    session: *mut InputiaSession,
    unicode_scalar: u32,
) -> *mut c_char {
    let Some(ch) = char::from_u32(unicode_scalar) else {
        return error_json("invalid Unicode scalar");
    };
    with_session(session, |session| session.core.handle_key(Key::Char(ch)))
}

/// 处理数字候选选择键，返回调用方所有的 JSON。
///
/// # Safety
/// 非空 session 必须仍存活且来自本库；在其所有者线程独占串行调用，不与任何 Rime 操作并发。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_handle_digit(
    session: *mut InputiaSession,
    digit: u8,
) -> *mut c_char {
    with_session(session, |session| {
        session.core.handle_key(Key::Digit(digit))
    })
}

/// 处理特殊键，返回调用方所有的 JSON。
///
/// # Safety
/// 非空 session 必须仍存活且来自本库；在其所有者线程独占串行调用，不与任何 Rime 操作并发。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_handle_special(
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

/// 读取 session 快照，返回调用方所有的 JSON。
///
/// # Safety
/// 非空 session 必须是本库仍存活的原始指针；读取也须独占串行，不能与更新/释放并发。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_snapshot(session: *mut InputiaSession) -> *mut c_char {
    if session.is_null() {
        return error_json("session is null");
    }
    let session = unsafe { &mut *session };
    outcome_json(OutputEnvelope::ok(None, false, session.core.snapshot()))
}

/// 仅在宿主确认自身最后一次提交确实被撤销后调用，不执行文本删除。
///
/// # Safety
/// session必须由当前线程独占且存活；不得用普通退格代替确认撤销。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_undo_recent_learning(
    session: *mut InputiaSession,
) -> *mut c_char {
    if session.is_null() {
        return error_json("session is null");
    }
    let session = unsafe { &*session };
    string_json(&serde_json::json!({"ok":true,"requested":session.core.undo_recent_learning()}))
}

/// 返回最多64个带稳定引擎身份的候选，不改变当前页。
///
/// # Safety
/// session 必须存活，调用必须位于唯一所有者线程且与其他Rime操作串行。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_candidate_pool(
    session: *mut InputiaSession,
    limit: usize,
) -> *mut c_char {
    if session.is_null() || limit == 0 || limit > 64 {
        return error_json("invalid candidate pool request");
    }
    let session = unsafe { &mut *session };
    let pool = session.core.candidate_pool(limit);
    let snapshot = session.core.snapshot();
    let candidates: Vec<_> = pool.into_iter().enumerate().map(|(rank, (c, consumed))| {
        let match_type = if c.id.starts_with("rime-correction:") { "correction" }
            else if consumed == snapshot.composing.len() { "exact" } else { "partial" };
        serde_json::json!({"id":c.id,"text":c.text,"base_rank":rank,"consumed_len":consumed,"match_type":match_type})
    }).collect();
    string_json(
        &serde_json::json!({"ok":true,"composing":snapshot.composing,"page":snapshot.page,"candidates":candidates}),
    )
}

/// 核验输入码、候选ID及显示正文后，回到真实引擎选择路径。
///
/// # Safety
/// 所有字符串指针必须有效且NUL结尾；session遵循唯一所有者及串行调用合同。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_choose_candidate_id(
    session: *mut InputiaSession,
    composing: *const c_char,
    candidate_id: *const c_char,
    expected_text: *const c_char,
) -> *mut c_char {
    if session.is_null() {
        return error_json("session is null");
    }
    let (Some(composing), Some(id), Some(text)) = (
        unsafe { optional_c_string(composing) },
        unsafe { optional_c_string(candidate_id) },
        unsafe { optional_c_string(expected_text) },
    ) else {
        return error_json("invalid candidate identity");
    };
    let session = unsafe { &mut *session };
    match session.core.choose_candidate_id(&composing, &id, &text) {
        Some(outcome) => outcome_json(OutputEnvelope::from_outcome(outcome)),
        None => error_json("candidate identity expired"),
    }
}

/// 只读内存快照，返回渲染顺序；选择仍使用原候选索引。
///
/// # Safety
/// session 必须存活且独占串行；terms_json 必须为有效 NUL 结尾 C 字符串。
/// 返回值由 inputia_string_free 释放。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_shared_candidate_order(
    session: *mut InputiaSession,
    terms_json: *const c_char,
) -> *mut c_char {
    if session.is_null() {
        return error_json("session is null");
    }
    if terms_json.is_null() {
        return error_json("invalid shared terms");
    }
    let bytes = unsafe { CStr::from_ptr(terms_json) }.to_bytes();
    if bytes.is_empty() || bytes.len() > 16_384 {
        return error_json("invalid shared terms");
    }
    let Ok(terms) = serde_json::from_slice::<Vec<String>>(bytes) else {
        return error_json("invalid shared terms");
    };
    if terms.is_empty() || terms.len() > 256 {
        return error_json("invalid shared terms");
    }
    let session = unsafe { &*session };
    let snapshot = session.core.snapshot();
    let indices = inputia_core::shared_candidate_order(
        &snapshot.composing,
        &snapshot.visible_candidates,
        &terms,
    );
    string_json(
        &serde_json::json!({"ok":true,"mode":mode_name(&snapshot.mode),"composing":snapshot.composing,"page":snapshot.page,"indices":indices}),
    )
}

/// 显式设置中英文模式。
///
/// # Safety
/// 非空 session 必须仍存活且来自本库；在其所有者线程独占串行调用，不与任何 Rime 操作并发。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_set_input_mode(
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

/// 设置应用上下文。
///
/// # Safety
/// 非空 session 必须独占、存活且来自本库；bundle_id 非空时须满足 C 字符串合同。
/// 调用须遵守模块的单一所有者线程和串行访问要求。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_set_app_context(
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
    session.context_verified.store(true, Ordering::Relaxed);
    session.context = AppContext::new(bundle_id);
    learning_json(LearningEnvelope::context_set())
}

/// 设置应用与可选窗口上下文。
///
/// # Safety
/// 非空 session 必须独占、存活且来自本库；两个非空字符串须满足 C 字符串合同。
/// window_title 可以为 null；调用须与其他 session 操作/释放串行。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_set_app_context_with_window(
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
    session.context_verified.store(true, Ordering::Relaxed);
    session.context = AppContext::new(bundle_id).with_window_title(window_title);
    learning_json(LearningEnvelope::context_set())
}

/// 未经主程序确认的上下文允许基本拼音，但禁止 Inputia 记忆读取、学习和重排。
///
/// # Safety
/// session 须为有效且独占的本库会话；bundle_id 须满足 C 字符串合同。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_set_context_unverified(
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
    session.context_verified.store(false, Ordering::Relaxed);
    session.context = AppContext::new(bundle_id);
    learning_json(LearningEnvelope::context_set())
}

/// 按既有隐私策略记录学习证据。
///
/// # Safety
/// 非空 session 必须独占、存活且来自本库；非空 text/bundle_id 必须满足 C 字符串合同。
/// 调用须在运行时所有者线程串行，返回 JSON 由调用方按模块合同回收。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_learn(
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
    if !session.context_verified.load(Ordering::Relaxed) {
        return learning_json(LearningEnvelope::error("context is not verified"));
    }
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

/// 按既有规则导入历史。
///
/// # Safety
/// 非空 session 必须独占、存活且来自本库；两个非空字符串须满足 C 字符串合同。
/// 导入与其他 session 操作/释放必须串行，返回 JSON 由调用方回收。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_import_handy_history(
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
    if !session.context_verified.load(Ordering::Relaxed) {
        return import_json(ImportEnvelope::error("context is not verified"));
    }
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

/// 按既有规则导入剪贴板记录。
///
/// # Safety
/// 非空 session 必须独占、存活且来自本库；两个非空字符串须满足 C 字符串合同。
/// 导入与其他 session 操作/释放必须串行，返回 JSON 由调用方回收。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_import_handy_clipboard(
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
    if !session.context_verified.load(Ordering::Relaxed) {
        return import_json(ImportEnvelope::error("context is not verified"));
    }
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

/// 获取语音热词 JSON。
///
/// # Safety
/// 非空 session 必须是本库仍存活的原始指针；读取也须独占串行，不能与更新/释放并发。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_voice_hotwords(
    session: *mut InputiaSession,
    limit: usize,
) -> *mut c_char {
    if session.is_null() {
        return hotwords_json(HotwordsEnvelope::error("session is null"));
    }
    let session = unsafe { &mut *session };
    if !session.context_verified.load(Ordering::Relaxed) {
        return hotwords_json(HotwordsEnvelope::error("context is not verified"));
    }
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

/// 获取剪贴板候选 JSON。
///
/// # Safety
/// 非空 session 必须是本库仍存活的原始指针；读取也须独占串行，不能与更新/释放并发。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_clipboard_candidates(
    session: *mut InputiaSession,
    limit: usize,
) -> *mut c_char {
    if session.is_null() {
        return candidate_list_json(CandidateListEnvelope::error("session is null"));
    }
    let session = unsafe { &mut *session };
    if !session.context_verified.load(Ordering::Relaxed) {
        return candidate_list_json(CandidateListEnvelope::error("context is not verified"));
    }
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

/// 获取英文补全候选 JSON。
///
/// # Safety
/// 非空 session 必须独占、存活且来自本库；非空 prefix 必须满足 C 字符串合同。
/// 调用须与其他 session 操作/释放串行，返回 JSON 由调用方回收。
#[no_mangle]
pub unsafe extern "C" fn inputia_session_completion_candidates(
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
    if !session.context_verified.load(Ordering::Relaxed) {
        return candidate_list_json(CandidateListEnvelope::error("context is not verified"));
    }
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

/// 释放本库分配的返回字符串；null 是无操作。
///
/// # Safety
/// 非空 value 必须是本库返回且尚未释放的原始指针，不得偏移、替换分配器或改变首个 NUL 位置。
/// 释放时不能再有借用/并发访问；同一分配只释放一次，不可使用 C free 或其他释放函数。
#[no_mangle]
pub unsafe extern "C" fn inputia_string_free(value: *mut c_char) {
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
    let config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir).with_schema(schema_id);
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
    if inputia_settings::maintenance::ensure_current_normal_start().is_err() {
        return null_mut();
    }
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
    let context_verified = Arc::new(AtomicBool::new(true));
    let ranked_engine = RankedRimeEngine {
        context_verified: context_verified.clone(),
        rime: engine,
        memory: memory.clone(),
    };
    let core = InputiaCore::new(options.core, ranked_engine);
    Box::into_raw(Box::new(InputiaSession {
        core,
        memory,
        context: AppContext::new("dev.inputia.host"),
        context_verified,
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
    [
        "/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/RimeData",
        "/Library/Input Methods/IputiaInputMethod.app/Contents/Resources/RimeData",
    ]
    .into_iter()
    .map(std::path::PathBuf::from)
    .find(|path| path.exists())
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
    if !session.context_verified.load(Ordering::Relaxed) {
        return;
    }
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
    // SAFETY: 调用方提供本次调用期间有效的 NUL 结尾字符串；null 已在上方处理。
    Some(
        unsafe { CStr::from_ptr(value) }
            .to_string_lossy()
            .into_owned(),
    )
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
    id: String,
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
            id: candidate.id,
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

    // SAFETY: 下方显式 unsafe 调用只使用存活 CString/本库 session/本库 JSON 指针。
    // RIME_CAPI_TEST_LOCK 串行化运行时，测试在最后使用后释放 session，handle_json 复制正文后释放返回值。

    #[test]
    fn ffi_exports_keep_c_abi_with_explicit_unsafe_contract() {
        let _: unsafe extern "C" fn(*const c_char, usize) -> *mut InputiaSession =
            super::inputia_session_new_luna_pinyin_simp;
        let _: unsafe extern "C" fn(
            *const c_char,
            *const c_char,
            *const c_char,
            *const c_char,
            usize,
        ) -> *mut InputiaSession = super::inputia_session_new_with_paths;
        let _: unsafe extern "C" fn(*mut InputiaSession, u32) -> *mut c_char =
            super::inputia_session_handle_char;
        let _: unsafe extern "C" fn(*mut InputiaSession) = super::inputia_session_free;
        let _: unsafe extern "C" fn(*mut c_char) = super::inputia_string_free;
    }

    #[test]
    fn ffi_null_contract_preserves_error_results_and_noop_free() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        // SAFETY: 本库为 null 明确保留错误/无操作合同；返回 JSON 是本库的独占分配。
        unsafe {
            assert!(super::inputia_session_new_luna_pinyin_simp(std::ptr::null(), 5).is_null());
            assert!(super::inputia_session_new_luna_pinyin_simp_with_memory(
                std::ptr::null(),
                std::ptr::null(),
                5
            )
            .is_null());
            assert!(
                super::inputia_session_new_with_schema(std::ptr::null(), std::ptr::null(), 5)
                    .is_null()
            );
            assert!(super::inputia_session_new_with_paths(
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                5
            )
            .is_null());
            assert!(super::inputia_session_new_from_settings(std::ptr::null()).is_null());
            assert!(
                super::inputia_session_new_from_settings_without_memory(std::ptr::null()).is_null()
            );
            for result in [
                super::inputia_session_handle_char(null_mut(), 'a' as u32),
                super::inputia_session_handle_digit(null_mut(), 1),
                super::inputia_session_handle_special(null_mut(), KEY_SPACE),
                super::inputia_session_snapshot(null_mut()),
                super::inputia_session_set_input_mode(null_mut(), INPUT_MODE_CHINESE),
                super::inputia_session_set_app_context(null_mut(), std::ptr::null()),
                super::inputia_session_set_app_context_with_window(
                    null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                ),
                super::inputia_session_learn(
                    null_mut(),
                    SOURCE_TYPED,
                    std::ptr::null(),
                    std::ptr::null(),
                ),
                super::inputia_session_import_handy_history(
                    null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    10,
                ),
                super::inputia_session_import_handy_clipboard(
                    null_mut(),
                    std::ptr::null(),
                    std::ptr::null(),
                    10,
                ),
                super::inputia_session_voice_hotwords(null_mut(), 10),
                super::inputia_session_clipboard_candidates(null_mut(), 10),
                super::inputia_session_completion_candidates(null_mut(), std::ptr::null(), 10),
            ] {
                assert_eq!(handle_json(result)["ok"], false);
            }
            super::inputia_session_free(null_mut());
            super::inputia_string_free(null_mut());
        }
    }

    fn unavailable(message: &str) {
        #[cfg(feature = "bundled-static-rime")]
        panic!("static CAPI test cannot skip: {message}");
        #[cfg(not(feature = "bundled-static-rime"))]
        eprintln!("{message}");
    }

    #[cfg(feature = "bundled-static-rime")]
    fn static_test_data() -> std::path::PathBuf {
        let path = std::path::PathBuf::from(
            std::env::var_os("INPUTIA_RIME_SHARED_DATA_DIR")
                .expect("static tests require explicit candidate INPUTIA_RIME_SHARED_DATA_DIR"),
        );
        assert!(path.is_absolute() && path.join("luna_pinyin_simp.schema.yaml").is_file());
        path.canonicalize().unwrap()
    }

    #[cfg(feature = "bundled-static-rime")]
    unsafe fn inputia_session_new_luna_pinyin_simp(
        user: *const c_char,
        count: usize,
    ) -> *mut InputiaSession {
        static_test_session(user, None, count)
    }

    #[cfg(feature = "bundled-static-rime")]
    unsafe fn inputia_session_new_luna_pinyin_simp_with_memory(
        user: *const c_char,
        memory: *const c_char,
        count: usize,
    ) -> *mut InputiaSession {
        static_test_session(user, Some(memory), count)
    }

    #[cfg(feature = "bundled-static-rime")]
    fn static_test_session(
        user: *const c_char,
        memory: Option<*const c_char>,
        count: usize,
    ) -> *mut InputiaSession {
        let user = std::path::PathBuf::from(unsafe { CStr::from_ptr(user) }.to_str().unwrap());
        let path = user.join("capi-static-test-settings.json");
        let settings = InputiaSettings {
            candidate_page_size: count,
            rime_user_data_dir: Some(user),
            rime_shared_data_dir: Some(static_test_data()),
            rime_dylib_path: Some("/synthetic/not-a-library.dylib".into()),
            memory_enabled: memory.is_some(),
            memory_db_path: memory.map(|pointer| {
                std::path::PathBuf::from(unsafe { CStr::from_ptr(pointer) }.to_str().unwrap())
            }),
            ..InputiaSettings::default()
        };
        settings.save(&path).unwrap();
        let path = CString::new(path.to_str().unwrap()).unwrap();
        unsafe { super::inputia_session_new_from_settings(path.as_ptr()) }
    }

    #[cfg(feature = "bundled-static-rime")]
    unsafe fn inputia_session_new_from_settings(path: *const c_char) -> *mut InputiaSession {
        let file = unsafe { CStr::from_ptr(path) }.to_str().unwrap();
        let mut settings = InputiaSettings::load(file).unwrap();
        settings.rime_shared_data_dir = Some(static_test_data());
        settings.rime_dylib_path = Some("/synthetic/not-a-library.dylib".into());
        settings.save(file).unwrap();
        unsafe { super::inputia_session_new_from_settings(path) }
    }

    #[cfg(feature = "bundled-static-rime")]
    fn default_inputia_shared_data_dir() -> Option<std::path::PathBuf> {
        Some(static_test_data())
    }

    #[cfg(feature = "bundled-static-rime")]
    fn bundled_shared_data_dir() -> Option<std::path::PathBuf> {
        Some(static_test_data())
    }

    #[test]
    #[cfg(feature = "bundled-static-rime")]
    fn personalization_pool_selects_beyond_visible_page_and_rejects_stale_text() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user = CString::new(temp.path().to_str().unwrap()).unwrap();
        let session = unsafe { inputia_session_new_luna_pinyin_simp(user.as_ptr(), 7) };
        assert!(!session.is_null());
        handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_CHINESE) });
        for c in "shi".chars() {
            handle_json(unsafe { inputia_session_handle_char(session, c as u32) });
        }
        let before = handle_json(unsafe { inputia_session_snapshot(session) });
        let pool = handle_json(unsafe { inputia_session_candidate_pool(session, 32) });
        let after = handle_json(unsafe { inputia_session_snapshot(session) });
        assert_eq!(
            before, after,
            "expanding pool must preserve the visible page"
        );
        let rows = pool["candidates"].as_array().unwrap();
        assert!(rows.len() > 7);
        let selected = &rows[10];
        let id = CString::new(selected["id"].as_str().unwrap()).unwrap();
        let text = CString::new(selected["text"].as_str().unwrap()).unwrap();
        let code = CString::new("shi").unwrap();
        let wrong = CString::new("不是显示的词").unwrap();
        let rejected = handle_json(unsafe {
            inputia_session_choose_candidate_id(session, code.as_ptr(), id.as_ptr(), wrong.as_ptr())
        });
        assert_eq!(rejected["ok"], false);
        let selected = handle_json(unsafe {
            inputia_session_choose_candidate_id(session, code.as_ptr(), id.as_ptr(), text.as_ptr())
        });
        assert_eq!(selected["commit"], text.to_str().unwrap());
        assert_eq!(selected["composing"], "");
        assert_eq!(
            handle_json(unsafe {
                inputia_session_choose_candidate_id(
                    session,
                    code.as_ptr(),
                    id.as_ptr(),
                    text.as_ptr(),
                )
            })["ok"],
            false
        );
        unsafe { inputia_session_free(session) };
    }

    #[test]
    #[cfg(feature = "bundled-static-rime")]
    fn native_rime_recent_learning_is_reversible_without_deleting_host_text() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let mut observations = Vec::new();
        let mut chosen = String::new();
        for undo in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let user = CString::new(temp.path().to_str().unwrap()).unwrap();
            let session = unsafe { inputia_session_new_luna_pinyin_simp(user.as_ptr(), 7) };
            assert!(!session.is_null());
            handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_CHINESE) });
            for c in "shijie".chars() {
                handle_json(unsafe { inputia_session_handle_char(session, c as u32) });
            }
            let pool = handle_json(unsafe { inputia_session_candidate_pool(session, 32) });
            let rows = pool["candidates"].as_array().unwrap();
            if chosen.is_empty() {
                chosen = rows
                    .iter()
                    .filter(|c| c["consumed_len"] == 6)
                    .nth(3)
                    .expect("full phrase candidate")["text"]
                    .as_str()
                    .unwrap()
                    .into();
            }
            let before = rows.iter().position(|c| c["text"] == chosen).unwrap();
            let item = &rows[before];
            let code = CString::new("shijie").unwrap();
            let id = CString::new(item["id"].as_str().unwrap()).unwrap();
            let text = CString::new(chosen.clone()).unwrap();
            let committed = handle_json(unsafe {
                inputia_session_choose_candidate_id(
                    session,
                    code.as_ptr(),
                    id.as_ptr(),
                    text.as_ptr(),
                )
            });
            assert_eq!(committed["commit"], chosen);
            if undo {
                assert_eq!(
                    handle_json(unsafe { inputia_session_undo_recent_learning(session) })
                        ["requested"],
                    true
                );
            }
            for c in "shijie".chars() {
                handle_json(unsafe { inputia_session_handle_char(session, c as u32) });
            }
            let after = handle_json(unsafe { inputia_session_candidate_pool(session, 32) });
            let rank = after["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .position(|c| c["text"] == chosen)
                .unwrap();
            observations.push((before, rank));
            unsafe { inputia_session_free(session) };
        }
        eprintln!("native_rime_learning before/after={observations:?}");
        assert!(
            observations[0].1 < observations[0].0,
            "native Rime selection should promote a full phrase"
        );
        assert!(
            observations[1].1 >= observations[0].1,
            "undo must not increase the learned preference"
        );
        assert_eq!(
            observations[1].1, observations[1].0,
            "native recent learning should roll back"
        );
    }

    #[test]
    #[cfg(feature = "bundled-static-rime")]
    fn personalization_feedback_has_engine_provided_consumption_for_full_pinyin() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user = CString::new(temp.path().to_str().unwrap()).unwrap();
        let session = unsafe { inputia_session_new_luna_pinyin_simp(user.as_ptr(), 7) };
        assert!(!session.is_null());
        handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_CHINESE) });
        for c in "liming".chars() {
            handle_json(unsafe { inputia_session_handle_char(session, c as u32) });
        }
        let pool = handle_json(unsafe { inputia_session_candidate_pool(session, 32) });
        assert!(
            pool["candidates"]
                .as_array()
                .unwrap()
                .iter()
                .any(|c| c["consumed_len"] == 6 && c["match_type"] == "exact"),
            "{pool}"
        );
        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_drives_core_with_rime_full_pinyin_when_available() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = CString::new(temp.path().to_string_lossy().as_bytes()).unwrap();
        let session = unsafe { inputia_session_new_luna_pinyin_simp(user_data_dir.as_ptr(), 2) };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let shift = handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) });
        assert_eq!(shift["mode"], "Chinese");

        let mut latest = shift;
        for ch in "zhongguo".chars() {
            latest = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        assert_eq!(latest["composing"], "zhongguo");
        assert_eq!(latest["visible_candidates"][0]["text"], "中国");

        let page_down =
            handle_json(unsafe { inputia_session_handle_special(session, KEY_PAGE_DOWN) });
        assert_eq!(page_down["page"], 1);
        assert_ne!(page_down["visible_candidates"][0]["text"], "中国");

        let page_up = handle_json(unsafe { inputia_session_handle_special(session, KEY_PAGE_UP) });
        assert_eq!(page_up["page"], 0);
        assert_eq!(page_up["visible_candidates"][0]["text"], "中国");

        let commit = handle_json(unsafe { inputia_session_handle_special(session, KEY_SPACE) });
        assert_eq!(commit["commit"], "中国");
        assert_eq!(commit["composing"], "");

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_enter_commits_raw_composition() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = CString::new(temp.path().to_string_lossy().as_bytes()).unwrap();
        let session = unsafe { inputia_session_new_luna_pinyin_simp(user_data_dir.as_ptr(), 5) };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        assert_eq!(
            handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) })["mode"],
            "Chinese"
        );
        for ch in "ni".chars() {
            let _ = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }

        let commit = handle_json(unsafe { inputia_session_handle_special(session, KEY_ENTER) });
        assert_eq!(commit["commit"], "ni");
        assert_eq!(commit["composing"], "");
        assert!(commit["visible_candidates"].as_array().unwrap().is_empty());

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_paginates_across_rime_candidate_pages() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = CString::new(temp.path().to_string_lossy().as_bytes()).unwrap();
        let session = unsafe { inputia_session_new_luna_pinyin_simp(user_data_dir.as_ptr(), 8) };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        assert_eq!(
            handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) })["mode"],
            "Chinese"
        );
        let mut latest = Value::Null;
        for ch in "ba".chars() {
            latest = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        assert_eq!(latest["visible_candidates"][0]["text"], "吧");
        assert!(!latest["visible_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["text"] == "叭"));

        let page_down =
            handle_json(unsafe { inputia_session_handle_special(session, KEY_PAGE_DOWN) });
        assert_eq!(page_down["page"], 1);
        assert_eq!(page_down["visible_candidates"].as_array().unwrap().len(), 8);
        assert!(
            page_down["visible_candidates"]
                .as_array()
                .unwrap()
                .iter()
                .any(|candidate| candidate["text"] == "叭"),
            "CAPI must preserve deeper Rime candidates such as 叭 instead of truncating RankedRimeEngine to its shallow default"
        );

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_can_open_double_pinyin_schema_when_prepared() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        #[cfg(feature = "bundled-static-rime")]
        let shared_data_dir = static_test_data();
        #[cfg(not(feature = "bundled-static-rime"))]
        let shared_data_dir = std::env::var_os("INPUTIA_RIME_SHARED_DATA_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("/tmp/inputia-rime-shared-double-pinyin"));
        let user_temp = tempfile::tempdir().unwrap();
        let user_data_dir = user_temp.path().to_path_buf();
        if !shared_data_dir
            .join("double_pinyin_flypy.schema.yaml")
            .exists()
        {
            unavailable("skip: run spikes/inputia-rime/prepare-double-pinyin-data.sh double_pinyin_flypy first");
            return;
        }

        let schema = CString::new("double_pinyin_flypy").unwrap();
        let dylib =
            CString::new("/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib")
                .unwrap();
        let shared = CString::new(shared_data_dir.to_string_lossy().as_bytes()).unwrap();
        let user = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let session = unsafe {
            inputia_session_new_with_paths(
                schema.as_ptr(),
                dylib.as_ptr(),
                shared.as_ptr(),
                user.as_ptr(),
                2,
            )
        };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let shift = handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) });
        assert_eq!(shift["mode"], "Chinese");

        let mut latest = shift;
        for ch in "vsgo".chars() {
            latest = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        assert_eq!(latest["composing"], "vsgo");
        assert_eq!(latest["visible_candidates"][0]["text"], "中国");

        let commit = handle_json(unsafe { inputia_session_handle_special(session, KEY_SPACE) });
        assert_eq!(commit["commit"], "中国");

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_memory_reranks_candidates_and_respects_sensitive_apps() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = temp.path().join("rime-user");
        let memory_db = temp.path().join("inputia-memory.db");
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let session = unsafe {
            inputia_session_new_luna_pinyin_simp_with_memory(
                user_data_dir.as_ptr(),
                memory_db.as_ptr(),
                5,
            )
        };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let source_app = CString::new("com.apple.TextEdit").unwrap();
        let remembered = CString::new("种过").unwrap();
        let learned = handle_json(unsafe {
            inputia_session_learn(
                session,
                SOURCE_CLIPBOARD,
                remembered.as_ptr(),
                source_app.as_ptr(),
            )
        });
        assert_eq!(learned["decision"], "learn");
        assert_eq!(learned["term"], "种过");

        let sensitive_app = CString::new("com.1password.1password").unwrap();
        let sensitive_term = CString::new("密码 候选").unwrap();
        let excluded = handle_json(unsafe {
            inputia_session_learn(
                session,
                SOURCE_CLIPBOARD,
                sensitive_term.as_ptr(),
                sensitive_app.as_ptr(),
            )
        });
        assert_eq!(excluded["decision"], "excluded");
        assert!(excluded["term"].is_null());

        let voice_term = CString::new("语音 热词").unwrap();
        let voice = handle_json(unsafe {
            inputia_session_learn(
                session,
                SOURCE_VOICE,
                voice_term.as_ptr(),
                source_app.as_ptr(),
            )
        });
        assert_eq!(voice["decision"], "learn");

        let clipboard_candidates =
            handle_json(unsafe { inputia_session_clipboard_candidates(session, 10) });
        assert_eq!(clipboard_candidates["candidates"][0]["text"], "种过");
        assert_eq!(clipboard_candidates["candidates"][0]["source"], "clipboard");
        assert!(!clipboard_candidates["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["text"] == "语音 热词"));

        let shift = handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) });
        assert_eq!(shift["mode"], "Chinese");

        let mut latest = shift;
        for ch in "zhongguo".chars() {
            latest = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        assert_eq!(latest["visible_candidates"][0]["text"], "种过");
        assert_eq!(latest["visible_candidates"][0]["source"], "clipboard");

        let commit = handle_json(unsafe { inputia_session_handle_special(session, KEY_SPACE) });
        assert_eq!(commit["commit"], "种过");

        let hotwords = handle_json(unsafe { inputia_session_voice_hotwords(session, 10) });
        let hotword_values = hotwords["hotwords"].as_array().unwrap();
        assert!(hotword_values.iter().any(|value| value == "语音 热词"));
        assert!(hotword_values.iter().any(|value| value == "种过"));
        assert!(!hotword_values.iter().any(|value| value == "密码 候选"));

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_long_double_pinyin_input_keeps_phrase_ahead_of_single_character_memory() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let Some(shared_data_dir) = bundled_shared_data_dir() else {
            unavailable("skip: Inputia bundled RimeData is not available");
            return;
        };

        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            schema_id: "double_pinyin".to_string(),
            candidate_page_size: 8,
            rime_shared_data_dir: Some(shared_data_dir),
            rime_user_data_dir: Some(temp.path().join("rime-user")),
            memory_db_path: Some(temp.path().join("inputia-memory.db")),
            memory_enabled: true,
            privacy_learning_enabled: true,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = unsafe { inputia_session_new_from_settings(settings_path.as_ptr()) };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let source_app = CString::new("com.apple.TextEdit").unwrap();
        let single_char = CString::new("你").unwrap();
        for _ in 0..20 {
            let learned = handle_json(unsafe {
                inputia_session_learn(
                    session,
                    SOURCE_TYPED,
                    single_char.as_ptr(),
                    source_app.as_ptr(),
                )
            });
            assert_eq!(learned["decision"], "learn");
        }

        assert_eq!(
            handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_CHINESE) })
                ["mode"],
            "Chinese"
        );
        let mut latest = Value::Null;
        for ch in "nilllema".chars() {
            latest = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }

        assert_eq!(latest["visible_candidates"][0]["text"], "你来了吗");
        let single_index = latest["visible_candidates"]
            .as_array()
            .unwrap()
            .iter()
            .position(|candidate| candidate["text"] == "你")
            .expect("single-character candidate should remain visible");
        let partial =
            handle_json(unsafe { inputia_session_handle_digit(session, (single_index + 1) as u8) });
        assert_eq!(partial["commit"], "你");
        assert_eq!(partial["composing"], "lllema");
        assert_eq!(partial["visible_candidates"][0]["text"], "来了吗");

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_returns_english_completion_candidates_from_typed_memory() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = temp.path().join("rime-user");
        let memory_db = temp.path().join("inputia-memory.db");
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let session = unsafe {
            inputia_session_new_luna_pinyin_simp_with_memory(
                user_data_dir.as_ptr(),
                memory_db.as_ptr(),
                5,
            )
        };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let source_app = CString::new("com.apple.TextEdit").unwrap();
        let inputia = CString::new("Inputia").unwrap();
        let input_layer = CString::new("input-layer").unwrap();
        assert_eq!(
            handle_json(unsafe {
                inputia_session_learn(session, SOURCE_TYPED, inputia.as_ptr(), source_app.as_ptr())
            })["decision"],
            "learn"
        );
        assert_eq!(
            handle_json(unsafe {
                inputia_session_learn(session, SOURCE_TYPED, inputia.as_ptr(), source_app.as_ptr())
            })["decision"],
            "learn"
        );
        assert_eq!(
            handle_json(unsafe {
                inputia_session_learn(
                    session,
                    SOURCE_CLIPBOARD,
                    input_layer.as_ptr(),
                    source_app.as_ptr(),
                )
            })["decision"],
            "learn"
        );

        let prefix = CString::new("in").unwrap();
        let completions = handle_json(unsafe {
            inputia_session_completion_candidates(session, prefix.as_ptr(), 5)
        });

        assert_eq!(completions["ok"], true);
        assert_eq!(completions["candidates"][0]["text"], "Inputia");
        assert_eq!(completions["candidates"][0]["source"], "english_completion");
        assert!(completions["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["text"] == "input-layer"));

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_imports_handy_history_into_voice_hotwords_and_ranking() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = temp.path().join("rime-user");
        let memory_db = temp.path().join("inputia-memory.db");
        let history_db = temp.path().join("history.db");
        create_handy_history_db(&history_db);
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let history_db = CString::new(history_db.to_string_lossy().as_bytes()).unwrap();
        let bundle_id = CString::new("com.pais.handy").unwrap();
        let session = unsafe {
            inputia_session_new_luna_pinyin_simp_with_memory(
                user_data_dir.as_ptr(),
                memory_db.as_ptr(),
                5,
            )
        };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let imported = handle_json(unsafe {
            inputia_session_import_handy_history(
                session,
                history_db.as_ptr(),
                bundle_id.as_ptr(),
                10,
            )
        });
        assert_eq!(imported["ok"], true);
        assert_eq!(imported["imported"], 2);

        let hotwords = handle_json(unsafe { inputia_session_voice_hotwords(session, 10) });
        let hotword_values = hotwords["hotwords"].as_array().unwrap();
        assert!(hotword_values.iter().any(|value| value == "种过"));
        assert!(hotword_values.iter().any(|value| value == "语音 热词"));

        assert_eq!(
            handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) })["mode"],
            "Chinese"
        );
        let mut latest = Value::Null;
        for ch in "zhongguo".chars() {
            latest = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        assert_eq!(latest["visible_candidates"][0]["text"], "种过");
        assert_eq!(latest["visible_candidates"][0]["source"], "voice");

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_imports_handy_clipboard_and_skips_sensitive_source_apps() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = temp.path().join("rime-user");
        let memory_db = temp.path().join("inputia-memory.db");
        let clipboard_db = temp.path().join("clipboard.db");
        create_handy_clipboard_db(&clipboard_db);
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let clipboard_db = CString::new(clipboard_db.to_string_lossy().as_bytes()).unwrap();
        let bundle_id = CString::new("com.pais.handy").unwrap();
        let session = unsafe {
            inputia_session_new_luna_pinyin_simp_with_memory(
                user_data_dir.as_ptr(),
                memory_db.as_ptr(),
                5,
            )
        };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let imported = handle_json(unsafe {
            inputia_session_import_handy_clipboard(
                session,
                clipboard_db.as_ptr(),
                bundle_id.as_ptr(),
                10,
            )
        });
        assert_eq!(imported["ok"], true);
        assert_eq!(imported["imported"], 1);

        let clipboard_candidates =
            handle_json(unsafe { inputia_session_clipboard_candidates(session, 10) });
        assert_eq!(clipboard_candidates["candidates"][0]["text"], "种过");
        assert_eq!(clipboard_candidates["candidates"][0]["source"], "clipboard");
        assert!(!clipboard_candidates["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .any(|candidate| candidate["text"] == "密码 候选"));

        assert_eq!(
            handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) })["mode"],
            "Chinese"
        );
        let mut latest = Value::Null;
        for ch in "zhongguo".chars() {
            latest = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        assert_eq!(latest["visible_candidates"][0]["text"], "种过");
        assert_eq!(latest["visible_candidates"][0]["source"], "clipboard");

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_unverified_context_types_without_memory_access() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = temp.path().join("rime-user");
        let memory_db = temp.path().join("inputia-memory.db");
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let session = unsafe {
            inputia_session_new_luna_pinyin_simp_with_memory(
                user_data_dir.as_ptr(),
                memory_db.as_ptr(),
                5,
            )
        };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let sensitive_app = CString::new("com.apple.TextEdit").unwrap();
        let context = handle_json(unsafe {
            inputia_session_set_context_unverified(session, sensitive_app.as_ptr())
        });
        assert_eq!(context["decision"], "context_set");

        let shift = handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) });
        assert_eq!(shift["mode"], "Chinese");

        for ch in "zhongguo".chars() {
            let _ = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        let commit = handle_json(unsafe { inputia_session_handle_special(session, KEY_SPACE) });
        assert_eq!(commit["commit"], "中国");

        let hotwords = handle_json(unsafe { inputia_session_voice_hotwords(session, 10) });
        assert_eq!(
            hotwords["ok"], false,
            "unverified context must not expose memory"
        );
        let text = CString::new("private-test-word").unwrap();
        let learned = handle_json(unsafe {
            inputia_session_learn(session, SOURCE_TYPED, text.as_ptr(), sensitive_app.as_ptr())
        });
        assert_eq!(learned["ok"], false);
        let restored = handle_json(unsafe {
            inputia_session_set_app_context(session, sensitive_app.as_ptr())
        });
        assert_eq!(restored["ok"], true);
        let restored_words = handle_json(unsafe { inputia_session_voice_hotwords(session, 20) });
        assert_eq!(restored_words["ok"], true);
        assert!(!restored_words["hotwords"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "中国" || v == "private-test-word"));

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_typed_commits_respect_current_app_context() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = temp.path().join("rime-user");
        let memory_db = temp.path().join("inputia-memory.db");
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let session = unsafe {
            inputia_session_new_luna_pinyin_simp_with_memory(
                user_data_dir.as_ptr(),
                memory_db.as_ptr(),
                5,
            )
        };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let sensitive_app = CString::new("com.1password.1password").unwrap();
        let context = handle_json(unsafe {
            inputia_session_set_app_context(session, sensitive_app.as_ptr())
        });
        assert_eq!(context["decision"], "context_set");

        let shift = handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) });
        assert_eq!(shift["mode"], "Chinese");

        for ch in "zhongguo".chars() {
            let _ = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        let commit = handle_json(unsafe { inputia_session_handle_special(session, KEY_SPACE) });
        assert_eq!(commit["commit"], "中国");

        let hotwords = handle_json(unsafe { inputia_session_voice_hotwords(session, 10) });
        let hotword_values = hotwords["hotwords"].as_array().unwrap();
        assert!(!hotword_values.iter().any(|value| value == "中国"));

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_window_contexts_can_block_learning() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = temp.path().join("rime-user");
        let memory_db = temp.path().join("inputia-memory.db");
        let user_data_dir = CString::new(user_data_dir.to_string_lossy().as_bytes()).unwrap();
        let memory_db = CString::new(memory_db.to_string_lossy().as_bytes()).unwrap();
        let session = unsafe {
            inputia_session_new_luna_pinyin_simp_with_memory(
                user_data_dir.as_ptr(),
                memory_db.as_ptr(),
                5,
            )
        };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let bundle_id = CString::new("com.apple.Safari").unwrap();
        let window_title = CString::new("Private Browsing - Bank Login").unwrap();
        let context = handle_json(unsafe {
            inputia_session_set_app_context_with_window(
                session,
                bundle_id.as_ptr(),
                window_title.as_ptr(),
            )
        });
        assert_eq!(context["decision"], "context_set");

        let secret = CString::new("secret phrase").unwrap();
        let learned = handle_json(unsafe {
            inputia_session_learn(session, SOURCE_TYPED, secret.as_ptr(), bundle_id.as_ptr())
        });
        assert_eq!(learned["decision"], "excluded");

        assert_eq!(
            handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) })["mode"],
            "Chinese"
        );
        for ch in "zhongguo".chars() {
            let _ = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        let commit = handle_json(unsafe { inputia_session_handle_special(session, KEY_SPACE) });
        assert_eq!(commit["commit"], "中国");

        let hotwords = handle_json(unsafe { inputia_session_voice_hotwords(session, 10) });
        let hotword_values = hotwords["hotwords"].as_array().unwrap();
        assert!(!hotword_values.iter().any(|value| value == "中国"));
        assert!(!hotword_values.iter().any(|value| value == "secret phrase"));

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_loads_shift_setting_from_settings_file() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            rime_user_data_dir: Some(temp.path().join("rime-user")),
            memory_enabled: false,
            shift_toggle_enabled: false,
            input_mode_toggle_shortcut: inputia_settings::InputModeToggleShortcut::None,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = unsafe { inputia_session_new_from_settings(settings_path.as_ptr()) };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let shift = handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) });

        assert_eq!(shift["consumed"], false);
        assert_eq!(shift["mode"], "English");

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_explicit_input_mode_toggle_supports_remapped_shortcut() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            rime_user_data_dir: Some(temp.path().join("rime-user")),
            memory_enabled: false,
            shift_toggle_enabled: false,
            input_mode_toggle_shortcut: inputia_settings::InputModeToggleShortcut::ControlSpace,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = unsafe { inputia_session_new_from_settings(settings_path.as_ptr()) };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let ignored_shift =
            handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) });
        assert_eq!(ignored_shift["mode"], "English");
        assert_eq!(ignored_shift["consumed"], false);

        let remapped_toggle =
            handle_json(unsafe { inputia_session_handle_special(session, KEY_TOGGLE_INPUT_MODE) });
        assert_eq!(remapped_toggle["mode"], "Chinese");
        assert_eq!(remapped_toggle["consumed"], true);

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_sets_input_mode_explicitly() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = CString::new(temp.path().to_string_lossy().as_bytes()).unwrap();
        let session = unsafe { inputia_session_new_luna_pinyin_simp(user_data_dir.as_ptr(), 5) };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let set_chinese =
            handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_CHINESE) });
        assert_eq!(set_chinese["mode"], "Chinese");
        assert_eq!(set_chinese["consumed"], false);

        let z = handle_json(unsafe { inputia_session_handle_char(session, 'z' as u32) });
        assert_eq!(z["mode"], "Chinese");
        assert_eq!(z["composing"], "z");

        let set_english =
            handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_ENGLISH) });
        assert_eq!(set_english["mode"], "English");

        let direct = handle_json(unsafe { inputia_session_handle_char(session, 'x' as u32) });
        assert_eq!(direct["mode"], "English");
        assert_eq!(direct["commit"], "x");

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_loads_candidate_count_and_punctuation_from_settings_file() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            rime_user_data_dir: Some(temp.path().join("rime-user")),
            memory_enabled: false,
            candidate_page_size: 2,
            punctuation_preference: inputia_settings::PunctuationPreference::FollowInputMode,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = unsafe { inputia_session_new_from_settings(settings_path.as_ptr()) };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let shift = handle_json(unsafe { inputia_session_handle_special(session, KEY_SHIFT) });
        assert_eq!(shift["mode"], "Chinese");

        let mut latest = shift;
        for ch in "zhongguo".chars() {
            latest = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        assert_eq!(latest["visible_candidates"].as_array().unwrap().len(), 2);

        let comma = handle_json(unsafe { inputia_session_handle_char(session, ',' as u32) });
        assert_eq!(comma["commit"], "，");

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn settings_fallback_without_memory_preserves_schema_and_candidate_count() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let Some(shared_data_dir) = default_inputia_shared_data_dir() else {
            unavailable("skip: Inputia RimeData is not installed on this machine");
            return;
        };
        if !shared_data_dir.join("double_pinyin.schema.yaml").exists() {
            unavailable("skip: double_pinyin schema is not available");
            return;
        }

        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let invalid_memory_path = temp.path().join("memory-as-directory");
        std::fs::create_dir_all(&invalid_memory_path).unwrap();
        let settings = InputiaSettings {
            schema_id: "double_pinyin".to_string(),
            candidate_page_size: 8,
            rime_shared_data_dir: Some(shared_data_dir),
            rime_user_data_dir: Some(temp.path().join("rime-user")),
            memory_enabled: true,
            privacy_learning_enabled: true,
            memory_db_path: Some(invalid_memory_path),
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();

        let failed = unsafe { inputia_session_new_from_settings(settings_path.as_ptr()) };
        assert!(failed.is_null());

        let session =
            unsafe { inputia_session_new_from_settings_without_memory(settings_path.as_ptr()) };
        assert!(!session.is_null());
        let _ = handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_CHINESE) });
        let mut latest = serde_json::Value::Null;
        for ch in "yh".chars() {
            latest = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        let candidates = latest["visible_candidates"].as_array().unwrap();
        assert_eq!(candidates.len(), 8);
        assert!(candidates.iter().any(|candidate| candidate["text"] == "洋"));

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_loads_full_width_setting_from_settings_file() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            rime_user_data_dir: Some(temp.path().join("rime-user")),
            memory_enabled: false,
            character_width_preference: inputia_settings::CharacterWidthPreference::FullWidth,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = unsafe { inputia_session_new_from_settings(settings_path.as_ptr()) };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let direct = handle_json(unsafe { inputia_session_handle_char(session, 'A' as u32) });
        assert_eq!(direct["mode"], "English");
        assert_eq!(direct["commit"], "Ａ");

        assert_eq!(
            handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_CHINESE) })
                ["mode"],
            "Chinese"
        );
        for ch in "ni".chars() {
            let _ = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        let raw = handle_json(unsafe { inputia_session_handle_special(session, KEY_ENTER) });
        assert_eq!(raw["commit"], "ｎｉ");

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn capi_loads_spelling_correction_setting_from_settings_file() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            rime_user_data_dir: Some(temp.path().join("rime-user")),
            memory_enabled: false,
            spelling_correction_enabled: true,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let session = unsafe { inputia_session_new_from_settings(settings_path.as_ptr()) };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        assert_eq!(
            handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_CHINESE) })
                ["mode"],
            "Chinese"
        );
        let mut latest = Value::Null;
        for ch in "dagn".chars() {
            latest = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        assert_eq!(latest["visible_candidates"][0]["text"], "当");

        unsafe { inputia_session_free(session) };
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
    fn capi_partial_double_pinyin_selection_preserves_configured_candidate_count() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let shared_data_dir = bundled_shared_data_dir().expect("部分选词回归需要实际 RimeData");
        for count in [7, 8] {
            let temp = tempfile::tempdir().unwrap();
            let settings_path = temp.path().join("settings.json");
            let settings = InputiaSettings {
                schema_id: "double_pinyin".into(),
                rime_shared_data_dir: Some(shared_data_dir.clone()),
                rime_user_data_dir: Some(temp.path().join("rime-user")),
                memory_enabled: false,
                candidate_page_size: count,
                ..InputiaSettings::default()
            };
            settings.save(&settings_path).unwrap();
            let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
            let session = unsafe { inputia_session_new_from_settings(settings_path.as_ptr()) };
            assert!(!session.is_null(), "部分选词回归必须初始化真实 Rime");
            handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_CHINESE) });
            let mut current = Value::Null;
            for ch in "nihkxd".chars() {
                current = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
            }
            assert_eq!(current["composing"], "nihkxd");
            let candidates = current["visible_candidates"].as_array().unwrap();
            let first = candidates
                .iter()
                .position(|candidate| candidate["text"] == "你好")
                .unwrap_or_else(|| panic!("nihkxd 的可见候选必须包含部分选择你好：{candidates:?}"));
            let partial =
                handle_json(unsafe { inputia_session_handle_digit(session, (first + 1) as u8) });
            assert_eq!(partial["commit"], "你好");
            assert_eq!(partial["composing"], "xd");
            let remainder = partial["visible_candidates"].as_array().unwrap();
            assert_eq!(
                remainder.len(),
                count,
                "部分选词后应保留配置的候选数：{remainder:?}"
            );
            // 隔离默认字频不同于用户词频；按真实引擎翻页，不要求箱固定排在首页。
            let mut page = partial.clone();
            let mut box_index = None;
            for _ in 0..10 {
                box_index = page["visible_candidates"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .position(|candidate| candidate["text"] == "箱");
                if box_index.is_some() {
                    break;
                }
                page =
                    handle_json(unsafe { inputia_session_handle_special(session, KEY_PAGE_DOWN) });
                assert_eq!(page["composing"], "xd");
            }
            let box_index =
                box_index.unwrap_or_else(|| panic!("xd 的真实引擎前十页必须可找到箱：{page:?}"));
            let chosen = handle_json(unsafe {
                inputia_session_handle_digit(session, (box_index + 1) as u8)
            });
            assert_eq!(chosen["commit"], "箱");
            assert_eq!(chosen["composing"], "");
            unsafe { inputia_session_free(session) };
        }
    }

    #[test]
    fn capi_settings_schemas_commit_zhongguo_when_available() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let Some(shared_data_dir) = bundled_shared_data_dir() else {
            unavailable("skip: Inputia bundled RimeData is not available");
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
            let session = unsafe { inputia_session_new_from_settings(settings_path.as_ptr()) };
            if session.is_null() {
                unavailable("skip: Squirrel librime runtime is not available");
                return;
            }

            assert_eq!(
                handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_CHINESE) })
                    ["mode"],
                "Chinese"
            );
            let mut latest = Value::Null;
            for ch in case.keys.chars() {
                latest = handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
            }
            assert_eq!(latest["composing"], case.keys, "{}", case.schema);
            assert_eq!(
                latest["visible_candidates"][0]["text"], "中国",
                "{} should rank 中国 first for {} through settings",
                case.schema, case.keys
            );
            assert_eq!(latest["visible_candidates"].as_array().unwrap().len(), 7);

            let commit = handle_json(unsafe { inputia_session_handle_special(session, KEY_SPACE) });
            assert_eq!(commit["commit"], "中国");
            assert_eq!(commit["composing"], "");
            unsafe { inputia_session_free(session) };
        }
    }

    #[test]
    fn capi_new_settings_session_survives_previous_session_free() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let Some(shared_data_dir) = bundled_shared_data_dir() else {
            unavailable("skip: Inputia bundled RimeData is not available");
            return;
        };

        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            schema_id: "double_pinyin".to_string(),
            rime_shared_data_dir: Some(shared_data_dir),
            rime_user_data_dir: Some(temp.path().join("rime-user")),
            memory_enabled: false,
            ..InputiaSettings::default()
        };
        settings.save(&settings_path).unwrap();
        let settings_path = CString::new(settings_path.to_string_lossy().as_bytes()).unwrap();
        let previous_session = unsafe { inputia_session_new_from_settings(settings_path.as_ptr()) };
        if previous_session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }
        let next_session = unsafe { inputia_session_new_from_settings(settings_path.as_ptr()) };
        assert!(!next_session.is_null());

        unsafe { inputia_session_free(previous_session) };

        assert_eq!(
            handle_json(unsafe {
                inputia_session_set_input_mode(next_session, INPUT_MODE_CHINESE)
            })["mode"],
            "Chinese"
        );
        let mut latest = Value::Null;
        for ch in "mlle".chars() {
            latest = handle_json(unsafe { inputia_session_handle_char(next_session, ch as u32) });
        }
        assert_eq!(latest["composing"], "mlle");
        assert_eq!(latest["visible_candidates"][0]["text"], "买了");

        unsafe { inputia_session_free(next_session) };
    }

    #[test]
    fn capi_toggles_punctuation_and_character_width_runtime() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let user_data_dir = CString::new(temp.path().to_string_lossy().as_bytes()).unwrap();
        let session = unsafe { inputia_session_new_luna_pinyin_simp(user_data_dir.as_ptr(), 5) };
        if session.is_null() {
            unavailable("skip: Squirrel librime runtime is not available");
            return;
        }

        let half_width = handle_json(unsafe { inputia_session_handle_char(session, 'A' as u32) });
        assert_eq!(half_width["commit"], "A");

        let toggle_width = handle_json(unsafe {
            inputia_session_handle_special(session, KEY_TOGGLE_CHARACTER_WIDTH)
        });
        assert_eq!(toggle_width["mode"], "English");
        assert_eq!(toggle_width["consumed"], true);

        let full_width = handle_json(unsafe { inputia_session_handle_char(session, 'A' as u32) });
        assert_eq!(full_width["commit"], "Ａ");

        assert_eq!(
            handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_CHINESE) })
                ["mode"],
            "Chinese"
        );
        let english_punctuation =
            handle_json(unsafe { inputia_session_handle_char(session, ',' as u32) });
        assert_eq!(english_punctuation["commit"], ",");

        let toggle_punctuation =
            handle_json(unsafe { inputia_session_handle_special(session, KEY_TOGGLE_PUNCTUATION) });
        assert_eq!(toggle_punctuation["mode"], "Chinese");
        assert_eq!(toggle_punctuation["consumed"], true);

        let chinese_punctuation =
            handle_json(unsafe { inputia_session_handle_char(session, ',' as u32) });
        assert_eq!(chinese_punctuation["commit"], "，");

        unsafe { inputia_session_free(session) };
    }

    #[test]
    fn shared_candidate_order_ffi_is_read_only_and_rejects_invalid_input() {
        let _guard = RIME_CAPI_TEST_LOCK.lock().unwrap();
        let terms = CString::new("[\"你好\"]").unwrap();
        assert_eq!(
            handle_json(unsafe {
                inputia_session_shared_candidate_order(null_mut(), terms.as_ptr())
            })["ok"],
            false
        );
        let temp = tempfile::tempdir().unwrap();
        let path = CString::new(temp.path().to_string_lossy().as_bytes()).unwrap();
        let session = unsafe { inputia_session_new_luna_pinyin_simp(path.as_ptr(), 5) };
        assert!(
            !session.is_null(),
            "共享排序验证需要真实 Rime 会话，不能跳过"
        );
        handle_json(unsafe { inputia_session_set_input_mode(session, INPUT_MODE_CHINESE) });
        for ch in "nihao".chars() {
            handle_json(unsafe { inputia_session_handle_char(session, ch as u32) });
        }
        let before = handle_json(unsafe { inputia_session_snapshot(session) });
        assert_eq!(before["mode"], "Chinese");
        assert_eq!(before["composing"], "nihao");
        let visible = before["visible_candidates"].as_array().unwrap();
        assert!(!visible.is_empty(), "真实 Rime Chinese 候选不能为空");
        let identity = (0..visible.len())
            .map(|i| serde_json::json!(i))
            .collect::<Vec<_>>();
        let promoted = visible.iter().enumerate().find_map(|(index, candidate)| {
            let text = candidate["text"].as_str().unwrap();
            inputia_core::integration::terms::validate_term(
                text,
                inputia_core::integration::terms::TermEvidence::ConfirmedCorrection,
            )
            .ok()?;
            let terms = CString::new(serde_json::to_string(&[text]).unwrap()).unwrap();
            let order = handle_json(unsafe {
                inputia_session_shared_candidate_order(session, terms.as_ptr())
            });
            let indices = order["indices"].as_array().unwrap();
            let display_index = indices
                .iter()
                .position(|value| value.as_u64() == Some(index as u64))
                .unwrap();
            (indices != &identity && display_index < index)
                .then(|| (index, text.to_owned(), display_index, order))
        });
        let (original_index, expected_commit, display_index, order) = promoted
            .unwrap_or_else(|| panic!("真实 Rime nihao 候选没有可合法提升的同组术语: {visible:?}"));
        assert_eq!(order["ok"], true);
        assert_ne!(order["indices"].as_array().unwrap(), &identity);
        for field in ["mode", "composing", "page"] {
            assert_eq!(order[field], before[field]);
        }
        let mut indices = order["indices"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i.as_u64().unwrap() as usize)
            .collect::<Vec<_>>();
        indices.sort_unstable();
        assert_eq!(
            indices,
            (0..before["visible_candidates"].as_array().unwrap().len()).collect::<Vec<_>>()
        );
        for invalid in ["", "null", "[]", "{}", "[1]", "not json"] {
            let invalid = CString::new(invalid).unwrap();
            assert_eq!(
                handle_json(unsafe {
                    inputia_session_shared_candidate_order(session, invalid.as_ptr())
                })["error"],
                "invalid shared terms"
            );
        }
        assert_eq!(
            handle_json(unsafe {
                inputia_session_shared_candidate_order(session, std::ptr::null())
            })["error"],
            "invalid shared terms"
        );
        assert_eq!(
            handle_json(unsafe { inputia_session_snapshot(session) }),
            before
        );
        let returned_original_index = order["indices"][display_index].as_u64().unwrap() as usize;
        assert_eq!(returned_original_index, original_index);
        let chosen = handle_json(unsafe {
            inputia_session_handle_digit(session, (returned_original_index + 1) as u8)
        });
        assert_eq!(chosen["commit"], expected_commit);
        unsafe { inputia_session_free(session) };
    }

    fn handle_json(raw: *mut c_char) -> Value {
        assert!(!raw.is_null());
        let text = unsafe { CStr::from_ptr(raw).to_string_lossy().into_owned() };
        unsafe { inputia_string_free(raw) };
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
                "种过",
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
                "种过",
                "hash-1",
                "种过",
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

    #[cfg(not(feature = "bundled-static-rime"))]
    fn bundled_shared_data_dir() -> Option<std::path::PathBuf> {
        if let Ok(path) = std::env::var("INPUTIA_RIME_SHARED_DATA_DIR") {
            let path = std::path::PathBuf::from(path);
            if path.exists() {
                return Some(path);
            }
        }

        [
            std::path::PathBuf::from(
                "/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/RimeData",
            ),
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../macos/InputiaInputMethod/build/RimeData"),
        ]
        .into_iter()
        .find(|path| path.exists())
    }
}
