use std::collections::HashSet;
use std::ffi::{CStr, CString};
use std::fmt;
use std::os::raw::{c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut, NonNull};
use std::sync::{Mutex, OnceLock};

use inputia_core::{Candidate, CandidateSelection, CandidateSource, ChineseEngine};
use libloading::Library;

type Bool = c_int;
type RimeSessionId = usize;
type UnusedFn = Option<unsafe extern "C" fn()>;

const FALSE: Bool = 0;
const TRUE: Bool = 1;
const INITIAL_CANDIDATE_TARGET: usize = 10;
const RIME_CANDIDATE_PAGE_SCAN_GUARD: usize = 256;
const DEFAULT_SQUIRREL_DYLIB: &str =
    "/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib";
const DEFAULT_SQUIRREL_SHARED_DATA: &str =
    "/Library/Input Methods/Squirrel.app/Contents/SharedSupport";

fn bool_to_rime(value: bool) -> Bool {
    if value {
        TRUE
    } else {
        FALSE
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RimeEngineConfig {
    pub dylib_path: PathBuf,
    pub shared_data_dir: PathBuf,
    pub user_data_dir: PathBuf,
    pub schema_id: String,
    pub output_options: Vec<(String, bool)>,
    pub spelling_correction: bool,
}

impl RimeEngineConfig {
    pub fn squirrel_luna_pinyin_simp(user_data_dir: impl Into<PathBuf>) -> Self {
        Self {
            dylib_path: PathBuf::from(DEFAULT_SQUIRREL_DYLIB),
            shared_data_dir: PathBuf::from(DEFAULT_SQUIRREL_SHARED_DATA),
            user_data_dir: user_data_dir.into(),
            schema_id: "luna_pinyin_simp".to_string(),
            output_options: vec![("simplification".to_string(), true)],
            spelling_correction: true,
        }
    }

    pub fn with_schema(mut self, schema_id: impl Into<String>) -> Self {
        self.schema_id = schema_id.into();
        self
    }

    pub fn with_shared_data_dir(mut self, shared_data_dir: impl Into<PathBuf>) -> Self {
        self.shared_data_dir = shared_data_dir.into();
        self
    }

    pub fn with_dylib_path(mut self, dylib_path: impl Into<PathBuf>) -> Self {
        self.dylib_path = dylib_path.into();
        self
    }

    pub fn with_spelling_correction(mut self, enabled: bool) -> Self {
        self.spelling_correction = enabled;
        self
    }

    pub fn with_output_options(mut self, options: Vec<(String, bool)>) -> Self {
        self.output_options = options;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RimeSnapshot {
    pub schema_id: String,
    pub preedit: String,
    pub page_no: i32,
    pub page_size: i32,
    pub is_last_page: bool,
    pub highlighted_candidate_index: i32,
    pub candidates: Vec<Candidate>,
    pub commit: Option<String>,
}

pub struct RimeEngine {
    _library: Library,
    api: NonNull<RimeApi>,
    config: RimeEngineConfig,
    cstrings: RimeCStringConfig,
    output_options: Vec<(CString, Bool)>,
    evaluation_lock: Mutex<()>,
    live_session: Mutex<Option<RimeLiveSession>>,
}

#[derive(Debug)]
struct RimeLiveSession {
    session_id: RimeSessionId,
    input: String,
    page: usize,
}

impl RimeEngine {
    pub fn open(config: RimeEngineConfig) -> Result<Self> {
        std::fs::create_dir_all(&config.user_data_dir)?;
        let library = unsafe { Library::new(&config.dylib_path) }?;
        let get_api =
            unsafe { library.get::<unsafe extern "C" fn() -> *mut RimeApi>(b"rime_get_api\0") }?;
        let api =
            NonNull::new(unsafe { get_api() }).ok_or(Error::Rime("rime_get_api returned null"))?;
        let cstrings = RimeCStringConfig::new(&config)?;
        let output_options = config
            .output_options
            .iter()
            .map(|(name, enabled)| Ok((CString::new(name.as_str())?, bool_to_rime(*enabled))))
            .collect::<Result<Vec<_>>>()?;

        let engine = Self {
            _library: library,
            api,
            config,
            cstrings,
            output_options,
            evaluation_lock: Mutex::new(()),
            live_session: Mutex::new(None),
        };
        engine.initialize()?;
        Ok(engine)
    }

    pub fn evaluate(&self, key_sequence: &str) -> Result<RimeSnapshot> {
        let _guard = self
            .evaluation_lock
            .lock()
            .map_err(|_| Error::LockPoisoned)?;
        let key_sequence = CString::new(key_sequence)?;
        let schema_id = CString::new(self.config.schema_id.as_str())?;
        let api = self.api();
        let create_session = required(api.create_session, "create_session")?;
        let destroy_session = required(api.destroy_session, "destroy_session")?;
        let select_schema = required(api.select_schema, "select_schema")?;
        let set_option = required(api.set_option, "set_option")?;
        let simulate_key_sequence = required(api.simulate_key_sequence, "simulate_key_sequence")?;

        let session_id = unsafe { create_session() };
        if session_id == 0 {
            return Err(Error::Rime("failed to create session"));
        }

        let result = (|| {
            if unsafe { select_schema(session_id, schema_id.as_ptr()) } == FALSE {
                return Err(Error::Rime("failed to select schema"));
            }

            for (output_option, enabled) in &self.output_options {
                unsafe { set_option(session_id, output_option.as_ptr(), *enabled) };
            }

            if unsafe { simulate_key_sequence(session_id, key_sequence.as_ptr()) } == FALSE {
                return Err(Error::Rime("failed to simulate key sequence"));
            }

            self.snapshot(session_id)
        })();

        unsafe { destroy_session(session_id) };
        result
    }

    pub fn evaluate_incremental(&self, composing: &str, page: usize) -> Result<RimeSnapshot> {
        let _guard = self
            .evaluation_lock
            .lock()
            .map_err(|_| Error::LockPoisoned)?;
        let api = self.api();
        let mut live_session = self.live_session.lock().map_err(|_| Error::LockPoisoned)?;
        let live_session = self.ensure_live_session(api, &mut live_session)?;

        self.prepare_live_input(api, live_session, composing)?;
        self.prepare_live_page(api, live_session, page)?;
        self.snapshot(live_session.session_id)
    }

    pub fn select_candidate(
        &self,
        composing: &str,
        _page: usize,
        _page_index: usize,
        candidate: &Candidate,
    ) -> Result<CandidateSelection> {
        let _guard = self
            .evaluation_lock
            .lock()
            .map_err(|_| Error::LockPoisoned)?;
        let address = rime_candidate_address(&candidate.id, &self.config.schema_id).ok_or(
            Error::Rime("candidate does not carry a selectable Rime address"),
        )?;
        let api = self.api();
        let mut live_session = self.live_session.lock().map_err(|_| Error::LockPoisoned)?;
        let live_session = self.ensure_live_session(api, &mut live_session)?;

        self.clear_live_session_composition(api, live_session)?;
        match address {
            RimeCandidateAddress::CurrentPage { page, page_index } => {
                let select_key = candidate_select_key(page_index).ok_or(Error::Rime(
                    "candidate page index is not selectable by digit",
                ))?;
                let key_sequence = selection_key_sequence(composing, page, select_key);
                self.simulate_key_sequence(api, live_session.session_id, &key_sequence)?;
            }
            RimeCandidateAddress::Global { index } => {
                self.simulate_key_sequence(api, live_session.session_id, composing)?;
                let select_candidate = required(api.select_candidate, "select_candidate")?;
                if unsafe { select_candidate(live_session.session_id, index) } == FALSE {
                    return Err(Error::Rime("failed to select candidate"));
                }
            }
        }

        let commit = candidate.text.clone();
        let get_input = required(api.get_input, "get_input")?;
        let rime_input = unsafe { c_string(get_input(live_session.session_id)) };
        let remaining = remaining_input_after_candidate_selection(
            &self.config.schema_id,
            composing,
            candidate,
            &rime_input,
        )
        .ok_or(Error::Rime(
            "failed to infer remaining input after selection",
        ))?;

        self.clear_live_session_composition(api, live_session)?;
        if !remaining.is_empty() {
            self.simulate_key_sequence(api, live_session.session_id, &remaining)?;
        }
        live_session.input = remaining.clone();
        live_session.page = 0;
        let candidates = if remaining.is_empty() {
            Vec::new()
        } else {
            self.snapshot(live_session.session_id)?.candidates
        };

        Ok(CandidateSelection {
            commit,
            composing: remaining,
            candidates,
        })
    }

    fn initialize(&self) -> Result<()> {
        let api = self.api();
        let signature = RimeRuntimeSignature::from(&self.config);
        let mut state = rime_runtime_state()
            .lock()
            .map_err(|_| Error::LockPoisoned)?;

        if state.active_count > 0 {
            if state.signature.as_ref() != Some(&signature) {
                return Err(Error::Rime(
                    "librime runtime is already initialized with different data paths",
                ));
            }
            state.active_count += 1;
            return Ok(());
        }

        let mut traits = self.cstrings.traits();
        let setup = required(api.setup, "setup")?;
        let initialize = required(api.initialize, "initialize")?;
        let start_maintenance = required(api.start_maintenance, "start_maintenance")?;
        let join_maintenance_thread =
            required(api.join_maintenance_thread, "join_maintenance_thread")?;

        unsafe {
            setup(&mut traits);
            initialize(&mut traits);
            if start_maintenance(FALSE) != FALSE {
                join_maintenance_thread();
            }
        }

        state.signature = Some(signature);
        state.active_count = 1;
        Ok(())
    }

    fn snapshot(&self, session_id: RimeSessionId) -> Result<RimeSnapshot> {
        let api = self.api();
        let get_current_schema = required(api.get_current_schema, "get_current_schema")?;
        let get_context = required(api.get_context, "get_context")?;
        let free_context = required(api.free_context, "free_context")?;

        let mut schema_buffer = [0_i8; 128];
        let schema_id = if unsafe {
            get_current_schema(session_id, schema_buffer.as_mut_ptr(), schema_buffer.len())
        } != FALSE
        {
            unsafe { c_string(schema_buffer.as_ptr()) }
        } else {
            self.config.schema_id.clone()
        };

        let mut context = RimeContext::initialized();
        if unsafe { get_context(session_id, &mut context) } == FALSE {
            return Err(Error::Rime("failed to get context"));
        }

        let snapshot = unsafe {
            let snapshot = RimeSnapshot {
                schema_id: schema_id.clone(),
                preedit: c_string(context.composition.preedit),
                page_no: context.menu.page_no,
                page_size: context.menu.page_size,
                is_last_page: context.menu.is_last_page != FALSE,
                highlighted_candidate_index: context.menu.highlighted_candidate_index,
                candidates: copy_candidates(&schema_id, &context.menu),
                commit: self.commit_text(session_id)?,
            };
            free_context(&mut context);
            snapshot
        };

        Ok(snapshot)
    }

    unsafe fn commit_text(&self, session_id: RimeSessionId) -> Result<Option<String>> {
        let api = self.api();
        let get_commit = required(api.get_commit, "get_commit")?;
        let free_commit = required(api.free_commit, "free_commit")?;
        let mut commit = RimeCommit::initialized();

        if get_commit(session_id, &mut commit) == FALSE {
            return Ok(None);
        }

        let text = c_string(commit.text);
        free_commit(&mut commit);
        if text.is_empty() {
            Ok(None)
        } else {
            Ok(Some(text))
        }
    }

    fn api(&self) -> &RimeApi {
        unsafe { self.api.as_ref() }
    }

    fn ensure_live_session<'a>(
        &self,
        api: &RimeApi,
        live_session: &'a mut Option<RimeLiveSession>,
    ) -> Result<&'a mut RimeLiveSession> {
        if live_session.is_none() {
            let create_session = required(api.create_session, "create_session")?;
            let select_schema = required(api.select_schema, "select_schema")?;
            let set_option = required(api.set_option, "set_option")?;
            let schema_id = CString::new(self.config.schema_id.as_str())?;

            let session_id = unsafe { create_session() };
            if session_id == 0 {
                return Err(Error::Rime("failed to create session"));
            }
            if unsafe { select_schema(session_id, schema_id.as_ptr()) } == FALSE {
                return Err(Error::Rime("failed to select schema"));
            }
            for (output_option, enabled) in &self.output_options {
                unsafe { set_option(session_id, output_option.as_ptr(), *enabled) };
            }
            *live_session = Some(RimeLiveSession {
                session_id,
                input: String::new(),
                page: 0,
            });
        }

        live_session
            .as_mut()
            .ok_or(Error::Rime("failed to retain live session"))
    }

    fn prepare_live_input(
        &self,
        api: &RimeApi,
        live_session: &mut RimeLiveSession,
        composing: &str,
    ) -> Result<()> {
        if live_session.input == composing {
            return Ok(());
        }

        if live_session.page == 0 && composing.starts_with(&live_session.input) {
            let suffix = &composing[live_session.input.len()..];
            self.simulate_key_sequence(api, live_session.session_id, suffix)?;
            live_session.input = composing.to_string();
            return Ok(());
        }

        self.clear_live_session_composition(api, live_session)?;
        if !composing.is_empty() {
            self.simulate_key_sequence(api, live_session.session_id, composing)?;
        }
        live_session.input = composing.to_string();
        live_session.page = 0;
        Ok(())
    }

    fn prepare_live_page(
        &self,
        api: &RimeApi,
        live_session: &mut RimeLiveSession,
        page: usize,
    ) -> Result<()> {
        while live_session.page < page {
            self.simulate_key_sequence(api, live_session.session_id, "{Page_Down}")?;
            live_session.page += 1;
        }
        while live_session.page > page {
            self.simulate_key_sequence(api, live_session.session_id, "{Page_Up}")?;
            live_session.page -= 1;
        }
        Ok(())
    }

    fn clear_live_session_composition(
        &self,
        api: &RimeApi,
        live_session: &mut RimeLiveSession,
    ) -> Result<()> {
        let clear_composition = required(api.clear_composition, "clear_composition")?;
        unsafe { clear_composition(live_session.session_id) };
        live_session.input.clear();
        live_session.page = 0;
        Ok(())
    }

    fn simulate_key_sequence(
        &self,
        api: &RimeApi,
        session_id: RimeSessionId,
        key_sequence: &str,
    ) -> Result<()> {
        if key_sequence.is_empty() {
            return Ok(());
        }
        let key_sequence = CString::new(key_sequence)?;
        let simulate_key_sequence = required(api.simulate_key_sequence, "simulate_key_sequence")?;
        if unsafe { simulate_key_sequence(session_id, key_sequence.as_ptr()) } == FALSE {
            return Err(Error::Rime("failed to simulate key sequence"));
        }
        Ok(())
    }
}

impl ChineseEngine for RimeEngine {
    fn candidates(&self, composing: &str) -> Vec<Candidate> {
        self.candidates_up_to(composing, INITIAL_CANDIDATE_TARGET)
    }

    fn candidates_up_to(&self, composing: &str, minimum_count: usize) -> Vec<Candidate> {
        let mut candidates = Vec::new();
        let mut seen_texts = HashSet::new();
        let minimum_count = minimum_count.max(1);

        if self.config.spelling_correction {
            for correction in spelling_correction_variants(composing) {
                self.append_candidates_until(
                    &correction,
                    true,
                    minimum_count,
                    &mut candidates,
                    &mut seen_texts,
                );
                if candidates.len() >= minimum_count {
                    return candidates;
                }
            }
        }
        self.append_candidates_until(
            composing,
            false,
            minimum_count,
            &mut candidates,
            &mut seen_texts,
        );

        candidates
    }

    fn candidate_consumed_len(&self, composing: &str, candidate: &Candidate) -> Option<usize> {
        candidate_consumed_len(&self.config.schema_id, composing, candidate)
    }

    fn select_candidate(
        &self,
        composing: &str,
        page: usize,
        page_index: usize,
        candidate: &Candidate,
    ) -> Option<CandidateSelection> {
        RimeEngine::select_candidate(self, composing, page, page_index, candidate).ok()
    }
}

impl RimeEngine {
    fn append_candidates_until(
        &self,
        composing: &str,
        corrected: bool,
        minimum_count: usize,
        candidates: &mut Vec<Candidate>,
        seen_texts: &mut HashSet<String>,
    ) {
        for page in 0..RIME_CANDIDATE_PAGE_SCAN_GUARD {
            let snapshot = if corrected {
                let key_sequence = paged_key_sequence(composing, page);
                self.evaluate(&key_sequence)
            } else {
                self.evaluate_incremental(composing, page)
            };
            let Ok(snapshot) = snapshot else {
                return;
            };

            if snapshot.candidates.is_empty() {
                break;
            }

            let mut added_candidates = 0;
            for mut candidate in snapshot.candidates {
                if !is_valid_candidate_text(&candidate.text) {
                    continue;
                }
                if !seen_texts.insert(candidate.text.clone()) {
                    continue;
                }
                added_candidates += 1;
                let global_index = candidates.len();
                let existing_id = candidate.id.clone();
                candidate.id = if corrected {
                    format!("rime-correction:{}:{global_index}", snapshot.schema_id)
                } else if existing_id.starts_with("rime:") {
                    existing_id
                } else {
                    format!("rime:{}:{global_index}", snapshot.schema_id)
                };
                if corrected && candidate.annotation.is_empty() {
                    candidate.annotation = format!("纠错: {composing}");
                }
                candidate.base_score = 1_000 - global_index as i32;
                candidates.push(candidate);
            }

            if candidates.len() >= minimum_count || snapshot.is_last_page || added_candidates == 0 {
                break;
            }
        }
    }
}

fn paged_key_sequence(composing: &str, page: usize) -> String {
    let mut key_sequence = composing.to_string();
    for _ in 0..page {
        key_sequence.push_str("{Page_Down}");
    }
    key_sequence
}

fn is_valid_candidate_text(text: &str) -> bool {
    !text.is_empty()
        && text.chars().all(|ch| {
            let value = ch as u32;
            ch != '\u{fffd}'
                && !ch.is_control()
                && !(0xe000..=0xf8ff).contains(&value)
                && !(0xf0000..=0xffffd).contains(&value)
                && !(0x100000..=0x10fffd).contains(&value)
                && !(0xfdd0..=0xfdef).contains(&value)
                && value & 0xffff != 0xfffe
                && value & 0xffff != 0xffff
        })
}

fn candidate_consumed_len(
    schema_id: &str,
    composing: &str,
    candidate: &Candidate,
) -> Option<usize> {
    if composing.is_empty() {
        return None;
    }

    let char_count = cjk_char_count(&candidate.text);
    if char_count == 0 {
        return None;
    }

    if composing.contains('\'') {
        return consumed_len_for_delimited_composition(composing, char_count);
    }

    if is_double_pinyin_schema(schema_id) {
        return Some((char_count * 2).min(composing.len()));
    }

    None
}

fn consumed_len_for_delimited_composition(composing: &str, char_count: usize) -> Option<usize> {
    let mut consumed_parts = 0;
    let mut consumed_len = 0;
    for part in composing.split_inclusive('\'') {
        consumed_len += part.len();
        if part.trim_matches('\'').is_empty() {
            continue;
        }
        consumed_parts += 1;
        if consumed_parts == char_count {
            return Some(consumed_len);
        }
    }
    Some(composing.len())
}

fn is_double_pinyin_schema(schema_id: &str) -> bool {
    schema_id == "double_pinyin"
        || schema_id == "guobiao_bispell"
        || schema_id.starts_with("double_pinyin_")
}

fn cjk_char_count(text: &str) -> usize {
    text.chars().filter(|ch| is_cjk_unified(*ch)).count()
}

fn is_cjk_unified(ch: char) -> bool {
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
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RimeCandidateAddress {
    CurrentPage { page: usize, page_index: usize },
    Global { index: usize },
}

fn rime_candidate_address(id: &str, schema_id: &str) -> Option<RimeCandidateAddress> {
    let mut parts = id.split(':');
    if parts.next()? != "rime" || parts.next()? != schema_id {
        return None;
    }
    let global_index = parts.next()?.parse::<usize>().ok()?;
    let Some(page) = parts.next() else {
        return Some(RimeCandidateAddress::Global {
            index: global_index,
        });
    };
    let page = page.parse::<usize>().ok()?;
    let page_index = parts.next()?.parse::<usize>().ok()?;
    Some(RimeCandidateAddress::CurrentPage { page, page_index })
}

fn candidate_select_key(page_index: usize) -> Option<char> {
    if page_index < 9 {
        Some(char::from(b'1' + page_index as u8))
    } else {
        None
    }
}

fn selection_key_sequence(composing: &str, page: usize, select_key: char) -> String {
    let mut sequence = paged_key_sequence(composing, page);
    sequence.push(select_key);
    sequence
}

fn remaining_input_after_candidate_selection(
    schema_id: &str,
    composing: &str,
    candidate: &Candidate,
    rime_input: &str,
) -> Option<String> {
    if rime_input.len() < composing.len() && composing.ends_with(rime_input) {
        return Some(rime_input.to_string());
    }

    let consumed_len = candidate_consumed_len(schema_id, composing, candidate)?;
    if consumed_len == 0
        || consumed_len > composing.len()
        || !composing.is_char_boundary(consumed_len)
    {
        return None;
    }
    Some(composing[consumed_len..].to_string())
}

fn spelling_correction_variants(input: &str) -> Vec<String> {
    if !input.chars().all(|ch| ch.is_ascii_lowercase()) || input.len() < 3 {
        return Vec::new();
    }

    let mut variants = Vec::new();
    let mut seen = HashSet::new();
    add_string_replacements(input, "gn", "ng", &mut variants, &mut seen);
    add_string_replacements(input, "oa", "ao", &mut variants, &mut seen);
    add_string_replacements(input, "ain", "ian", &mut variants, &mut seen);
    add_string_replacements(input, "aing", "iang", &mut variants, &mut seen);
    add_string_replacements(input, "aio", "iao", &mut variants, &mut seen);
    add_string_replacements(input, "aun", "uan", &mut variants, &mut seen);
    add_string_replacements(input, "aung", "uang", &mut variants, &mut seen);
    add_ong_boundary_corrections(input, &mut variants, &mut seen);
    variants
}

fn add_string_replacements(
    input: &str,
    from: &str,
    to: &str,
    variants: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    for (index, _) in input.match_indices(from) {
        let mut variant = String::with_capacity(input.len() + to.len().saturating_sub(from.len()));
        variant.push_str(&input[..index]);
        variant.push_str(to);
        variant.push_str(&input[index + from.len()..]);
        add_variant(input, variant, variants, seen);
    }
}

fn add_ong_boundary_corrections(
    input: &str,
    variants: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    for (index, _) in input.match_indices("ong") {
        let next_index = index + 3;
        let Some(next) = input.as_bytes().get(next_index).copied() else {
            continue;
        };
        if !matches!(next, b'a' | b'e' | b'i' | b'o' | b'u') {
            continue;
        }

        let mut variant = String::with_capacity(input.len() + 1);
        variant.push_str(&input[..next_index]);
        variant.push('g');
        variant.push_str(&input[next_index..]);
        add_variant(input, variant, variants, seen);
    }
}

fn add_variant(
    input: &str,
    variant: String,
    variants: &mut Vec<String>,
    seen: &mut HashSet<String>,
) {
    if variant != input && seen.insert(variant.clone()) {
        variants.push(variant);
    }
}

impl Drop for RimeEngine {
    fn drop(&mut self) {
        if let Ok(mut live_session) = self.live_session.lock() {
            if let Some(live_session) = live_session.take() {
                if let Some(destroy_session) = self.api().destroy_session {
                    unsafe { destroy_session(live_session.session_id) };
                }
            }
        }
        release_rime_runtime(self.api(), &self.config);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RimeRuntimeSignature {
    dylib_path: PathBuf,
    shared_data_dir: PathBuf,
    user_data_dir: PathBuf,
}

impl From<&RimeEngineConfig> for RimeRuntimeSignature {
    fn from(config: &RimeEngineConfig) -> Self {
        Self {
            dylib_path: config.dylib_path.clone(),
            shared_data_dir: config.shared_data_dir.clone(),
            user_data_dir: config.user_data_dir.clone(),
        }
    }
}

#[derive(Default)]
struct RimeRuntimeState {
    active_count: usize,
    signature: Option<RimeRuntimeSignature>,
}

static RIME_RUNTIME_STATE: OnceLock<Mutex<RimeRuntimeState>> = OnceLock::new();

fn rime_runtime_state() -> &'static Mutex<RimeRuntimeState> {
    RIME_RUNTIME_STATE.get_or_init(|| Mutex::new(RimeRuntimeState::default()))
}

fn release_rime_runtime(api: &RimeApi, config: &RimeEngineConfig) {
    let Ok(mut state) = rime_runtime_state().lock() else {
        return;
    };
    if state.active_count == 0 {
        return;
    }
    if state.signature.as_ref() != Some(&RimeRuntimeSignature::from(config)) {
        return;
    }

    state.active_count -= 1;
    if state.active_count == 0 {
        if let Some(finalize) = api.finalize {
            unsafe { finalize() };
        }
        state.signature = None;
    }
}

fn required<T: Copy>(function: Option<T>, name: &'static str) -> Result<T> {
    function.ok_or(Error::ApiUnavailable(name))
}

unsafe fn copy_candidates(schema_id: &str, menu: &RimeMenu) -> Vec<Candidate> {
    if menu.candidates.is_null() || menu.num_candidates <= 0 {
        return Vec::new();
    }

    let page = menu.page_no.max(0) as usize;
    let page_size = menu.page_size.max(1) as usize;
    (0..menu.num_candidates)
        .map(|index| {
            let raw_candidate = &*menu.candidates.add(index as usize);
            let text = c_string(raw_candidate.text);
            let page_index = index as usize;
            let global_index = page * page_size + page_index;
            let mut candidate = Candidate::new(
                format!("rime:{schema_id}:{global_index}:{page}:{page_index}"),
                text,
            );
            candidate.annotation = c_string(raw_candidate.comment);
            candidate.source = CandidateSource::Engine;
            candidate.base_score = 1_000 - global_index as i32;
            candidate
        })
        .collect()
}

unsafe fn c_string(pointer: *const c_char) -> String {
    if pointer.is_null() {
        String::new()
    } else {
        CStr::from_ptr(pointer).to_string_lossy().into_owned()
    }
}

fn rime_data_size<T>() -> c_int {
    (std::mem::size_of::<T>() - std::mem::size_of::<c_int>()) as c_int
}

struct RimeCStringConfig {
    shared_data_dir: CString,
    user_data_dir: CString,
    distribution_name: CString,
    distribution_code_name: CString,
    distribution_version: CString,
    app_name: CString,
    log_dir: CString,
}

impl RimeCStringConfig {
    fn new(config: &RimeEngineConfig) -> Result<Self> {
        Ok(Self {
            shared_data_dir: path_to_cstring(&config.shared_data_dir)?,
            user_data_dir: path_to_cstring(&config.user_data_dir)?,
            distribution_name: CString::new("Inputia")?,
            distribution_code_name: CString::new("inputia")?,
            distribution_version: CString::new("0.0.1")?,
            app_name: CString::new("rime.inputia")?,
            log_dir: CString::new("")?,
        })
    }

    fn traits(&self) -> RimeTraits {
        RimeTraits {
            data_size: rime_data_size::<RimeTraits>(),
            shared_data_dir: self.shared_data_dir.as_ptr(),
            user_data_dir: self.user_data_dir.as_ptr(),
            distribution_name: self.distribution_name.as_ptr(),
            distribution_code_name: self.distribution_code_name.as_ptr(),
            distribution_version: self.distribution_version.as_ptr(),
            app_name: self.app_name.as_ptr(),
            modules: null(),
            min_log_level: 2,
            log_dir: self.log_dir.as_ptr(),
            prebuilt_data_dir: null(),
            staging_dir: null(),
        }
    }
}

fn path_to_cstring(path: &Path) -> Result<CString> {
    CString::new(path.to_string_lossy().as_bytes()).map_err(Error::from)
}

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub enum Error {
    ApiUnavailable(&'static str),
    Io(std::io::Error),
    Library(libloading::Error),
    LockPoisoned,
    Nul(std::ffi::NulError),
    Rime(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiUnavailable(name) => write!(formatter, "librime API unavailable: {name}"),
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Library(error) => write!(formatter, "library loading error: {error}"),
            Self::LockPoisoned => write!(formatter, "Rime evaluation lock was poisoned"),
            Self::Nul(error) => write!(formatter, "string contains interior NUL: {error}"),
            Self::Rime(message) => write!(formatter, "librime error: {message}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<libloading::Error> for Error {
    fn from(error: libloading::Error) -> Self {
        Self::Library(error)
    }
}

impl From<std::ffi::NulError> for Error {
    fn from(error: std::ffi::NulError) -> Self {
        Self::Nul(error)
    }
}

#[repr(C)]
struct RimeTraits {
    data_size: c_int,
    shared_data_dir: *const c_char,
    user_data_dir: *const c_char,
    distribution_name: *const c_char,
    distribution_code_name: *const c_char,
    distribution_version: *const c_char,
    app_name: *const c_char,
    modules: *const *const c_char,
    min_log_level: c_int,
    log_dir: *const c_char,
    prebuilt_data_dir: *const c_char,
    staging_dir: *const c_char,
}

#[repr(C)]
struct RimeComposition {
    length: c_int,
    cursor_pos: c_int,
    sel_start: c_int,
    sel_end: c_int,
    preedit: *mut c_char,
}

#[repr(C)]
struct RimeCandidate {
    text: *mut c_char,
    comment: *mut c_char,
    reserved: *mut c_void,
}

#[repr(C)]
struct RimeMenu {
    page_size: c_int,
    page_no: c_int,
    is_last_page: Bool,
    highlighted_candidate_index: c_int,
    num_candidates: c_int,
    candidates: *mut RimeCandidate,
    select_keys: *mut c_char,
}

#[repr(C)]
struct RimeCommit {
    data_size: c_int,
    text: *mut c_char,
}

impl RimeCommit {
    fn initialized() -> Self {
        Self {
            data_size: rime_data_size::<Self>(),
            text: null_mut(),
        }
    }
}

#[repr(C)]
struct RimeContext {
    data_size: c_int,
    composition: RimeComposition,
    menu: RimeMenu,
    commit_text_preview: *mut c_char,
    select_labels: *mut *mut c_char,
}

impl RimeContext {
    fn initialized() -> Self {
        Self {
            data_size: rime_data_size::<Self>(),
            composition: RimeComposition {
                length: 0,
                cursor_pos: 0,
                sel_start: 0,
                sel_end: 0,
                preedit: null_mut(),
            },
            menu: RimeMenu {
                page_size: 0,
                page_no: 0,
                is_last_page: FALSE,
                highlighted_candidate_index: 0,
                num_candidates: 0,
                candidates: null_mut(),
                select_keys: null_mut(),
            },
            commit_text_preview: null_mut(),
            select_labels: null_mut(),
        }
    }
}

#[repr(C)]
struct RimeApi {
    data_size: c_int,
    setup: Option<unsafe extern "C" fn(*mut RimeTraits)>,
    set_notification_handler: UnusedFn,
    initialize: Option<unsafe extern "C" fn(*mut RimeTraits)>,
    finalize: Option<unsafe extern "C" fn()>,
    start_maintenance: Option<unsafe extern "C" fn(Bool) -> Bool>,
    is_maintenance_mode: UnusedFn,
    join_maintenance_thread: Option<unsafe extern "C" fn()>,
    deployer_initialize: UnusedFn,
    prebuild: UnusedFn,
    deploy: UnusedFn,
    deploy_schema: UnusedFn,
    deploy_config_file: UnusedFn,
    sync_user_data: UnusedFn,
    create_session: Option<unsafe extern "C" fn() -> RimeSessionId>,
    find_session: UnusedFn,
    destroy_session: Option<unsafe extern "C" fn(RimeSessionId) -> Bool>,
    cleanup_stale_sessions: UnusedFn,
    cleanup_all_sessions: UnusedFn,
    process_key: Option<unsafe extern "C" fn(RimeSessionId, c_int, c_int) -> Bool>,
    commit_composition: Option<unsafe extern "C" fn(RimeSessionId) -> Bool>,
    clear_composition: Option<unsafe extern "C" fn(RimeSessionId)>,
    get_commit: Option<unsafe extern "C" fn(RimeSessionId, *mut RimeCommit) -> Bool>,
    free_commit: Option<unsafe extern "C" fn(*mut RimeCommit) -> Bool>,
    get_context: Option<unsafe extern "C" fn(RimeSessionId, *mut RimeContext) -> Bool>,
    free_context: Option<unsafe extern "C" fn(*mut RimeContext) -> Bool>,
    get_status: UnusedFn,
    free_status: UnusedFn,
    set_option: Option<unsafe extern "C" fn(RimeSessionId, *const c_char, Bool)>,
    get_option: UnusedFn,
    set_property: UnusedFn,
    get_property: UnusedFn,
    get_schema_list: UnusedFn,
    free_schema_list: UnusedFn,
    get_current_schema: Option<unsafe extern "C" fn(RimeSessionId, *mut c_char, usize) -> Bool>,
    select_schema: Option<unsafe extern "C" fn(RimeSessionId, *const c_char) -> Bool>,
    schema_open: UnusedFn,
    config_open: UnusedFn,
    config_close: UnusedFn,
    config_get_bool: UnusedFn,
    config_get_int: UnusedFn,
    config_get_double: UnusedFn,
    config_get_string: UnusedFn,
    config_get_cstring: UnusedFn,
    config_update_signature: UnusedFn,
    config_begin_map: UnusedFn,
    config_next: UnusedFn,
    config_end: UnusedFn,
    simulate_key_sequence: Option<unsafe extern "C" fn(RimeSessionId, *const c_char) -> Bool>,
    register_module: UnusedFn,
    find_module: UnusedFn,
    run_task: UnusedFn,
    get_shared_data_dir: UnusedFn,
    get_user_data_dir: UnusedFn,
    get_sync_dir: UnusedFn,
    get_user_id: UnusedFn,
    get_user_data_sync_dir: UnusedFn,
    config_init: UnusedFn,
    config_load_string: UnusedFn,
    config_set_bool: UnusedFn,
    config_set_int: UnusedFn,
    config_set_double: UnusedFn,
    config_set_string: UnusedFn,
    config_get_item: UnusedFn,
    config_set_item: UnusedFn,
    config_clear: UnusedFn,
    config_create_list: UnusedFn,
    config_create_map: UnusedFn,
    config_list_size: UnusedFn,
    config_begin_list: UnusedFn,
    get_input: Option<unsafe extern "C" fn(RimeSessionId) -> *const c_char>,
    get_caret_pos: UnusedFn,
    select_candidate: Option<unsafe extern "C" fn(RimeSessionId, usize) -> Bool>,
    get_version: UnusedFn,
    set_caret_pos: UnusedFn,
    select_candidate_on_current_page: Option<unsafe extern "C" fn(RimeSessionId, usize) -> Bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn squirrel_default_config_keeps_rime_outside_core() {
        let config = RimeEngineConfig::squirrel_luna_pinyin_simp("/tmp/inputia-rime-test");

        assert_eq!(config.schema_id, "luna_pinyin_simp");
        assert!(config.dylib_path.ends_with("librime.1.dylib"));
        assert!(config.shared_data_dir.ends_with("SharedSupport"));
        assert!(config.spelling_correction);
        assert_eq!(
            config.user_data_dir,
            PathBuf::from("/tmp/inputia-rime-test")
        );
    }

    #[test]
    fn spelling_correction_variants_cover_common_pinyin_typos() {
        assert!(spelling_correction_variants("dagn").contains(&"dang".to_string()));
        assert!(spelling_correction_variants("hoa").contains(&"hao".to_string()));
        assert!(spelling_correction_variants("tain").contains(&"tian".to_string()));
        assert!(spelling_correction_variants("zhonguo").contains(&"zhongguo".to_string()));
        assert!(spelling_correction_variants("zhongguo").is_empty());
    }

    #[test]
    fn candidate_text_keeps_unicode_han_and_rejects_invalid_scalars() {
        assert!(is_valid_candidate_text("逻辑𤓓𨱍𫗩𰻞"));
        assert!(!is_valid_candidate_text("\u{e000}"));
        assert!(!is_valid_candidate_text("\u{fffd}"));
        assert!(!is_valid_candidate_text("候选\u{0007}"));
        assert!(!is_valid_candidate_text("\u{fdd0}"));
    }
}
