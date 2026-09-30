#[cfg(feature = "sqlite-memory")]
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputMode {
    English,
    Chinese,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PunctuationPreference {
    FollowInputMode,
    EnglishInChinese,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CharacterWidthPreference {
    HalfWidth,
    FullWidth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CoreSettings {
    pub candidate_page_size: usize,
    pub shift_toggle_enabled: bool,
    pub punctuation_preference: PunctuationPreference,
    pub character_width_preference: CharacterWidthPreference,
}

impl Default for CoreSettings {
    fn default() -> Self {
        Self {
            candidate_page_size: 7,
            shift_toggle_enabled: true,
            punctuation_preference: PunctuationPreference::EnglishInChinese,
            character_width_preference: CharacterWidthPreference::HalfWidth,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Candidate {
    pub id: String,
    pub text: String,
    pub annotation: String,
    pub source: CandidateSource,
    pub base_score: i32,
    pub memory_score: i32,
}

impl Candidate {
    pub fn new(id: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            annotation: String::new(),
            source: CandidateSource::Engine,
            base_score: 0,
            memory_score: 0,
        }
    }

    pub fn final_score(&self) -> i32 {
        self.base_score + self.memory_score
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CandidateCommit {
    pub text: Option<String>,
    pub remaining_composing: String,
    pub preedit: String,
    pub page: usize,
    pub candidates: Vec<Candidate>,
}

impl CandidateCommit {
    pub fn new(
        text: impl Into<String>,
        remaining_composing: impl Into<String>,
        page: usize,
        candidates: Vec<Candidate>,
    ) -> Self {
        Self {
            text: Some(text.into()),
            remaining_composing: remaining_composing.into(),
            preedit: String::new(),
            page,
            candidates,
        }
    }

    pub fn preedit(
        preedit: impl Into<String>,
        remaining_composing: impl Into<String>,
        page: usize,
        candidates: Vec<Candidate>,
    ) -> Self {
        Self {
            text: None,
            remaining_composing: remaining_composing.into(),
            preedit: preedit.into(),
            page,
            candidates,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CandidateSource {
    Engine,
    Memory,
    Clipboard,
    Voice,
    EnglishCompletion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Key {
    Char(char),
    Digit(u8),
    Backspace,
    Escape,
    Space,
    Enter,
    Shift,
    PageDown,
    PageUp,
    ToggleInputMode,
    TogglePunctuation,
    ToggleCharacterWidth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputSnapshot {
    pub mode: InputMode,
    pub composing: String,
    pub page: usize,
    pub page_size: usize,
    pub visible_candidates: Vec<Candidate>,
    pub panel_candidates: Vec<Candidate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InputOutcome {
    pub consumed: bool,
    pub commit: Option<String>,
    pub snapshot: InputSnapshot,
}

pub trait ChineseEngine {
    fn candidates(&self, composing: &str) -> Vec<Candidate>;

    fn select_candidate(
        &self,
        _composing: &str,
        _candidate: &Candidate,
        _page: usize,
        _page_index: usize,
    ) -> Option<CandidateCommit> {
        None
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum MemorySource {
    Typed,
    Voice,
    Clipboard,
}

impl MemorySource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Typed => "typed",
            Self::Voice => "voice",
            Self::Clipboard => "clipboard",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppContext {
    pub bundle_id: String,
    pub window_title: Option<String>,
    pub document_id: Option<String>,
}

impl AppContext {
    pub fn new(bundle_id: impl Into<String>) -> Self {
        Self {
            bundle_id: bundle_id.into(),
            window_title: None,
            document_id: None,
        }
    }

    pub fn with_window_title(mut self, window_title: impl Into<Option<String>>) -> Self {
        self.window_title = window_title.into();
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PrivacyDecision {
    Learn,
    Excluded,
}

impl PrivacyDecision {
    #[cfg(feature = "sqlite-memory")]
    fn as_str(&self) -> &'static str {
        match self {
            Self::Learn => "learn",
            Self::Excluded => "excluded",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LearningOutcome {
    pub decision: PrivacyDecision,
    pub term: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RankerWeights {
    pub rime_rank: f32,
    pub typed: f32,
    pub voice: f32,
    pub clipboard: f32,
    pub recency: f32,
    pub app_context: f32,
    pub selection_feedback: f32,
}

impl Default for RankerWeights {
    fn default() -> Self {
        Self {
            rime_rank: 1.0,
            typed: 60.0,
            voice: 35.0,
            clipboard: 20.0,
            recency: 1.0,
            app_context: 40.0,
            selection_feedback: 80.0,
        }
    }
}

impl RankerWeights {
    const RIME_RANK_BOUNDS: (f32, f32) = (0.0, 2.0);
    const MEMORY_BOUNDS: (f32, f32) = (0.0, 200.0);

    pub fn clamped(self) -> Self {
        Self {
            rime_rank: clamp_weight(self.rime_rank, Self::RIME_RANK_BOUNDS),
            typed: clamp_weight(self.typed, Self::MEMORY_BOUNDS),
            voice: clamp_weight(self.voice, Self::MEMORY_BOUNDS),
            clipboard: clamp_weight(self.clipboard, Self::MEMORY_BOUNDS),
            recency: clamp_weight(self.recency, Self::MEMORY_BOUNDS),
            app_context: clamp_weight(self.app_context, Self::MEMORY_BOUNDS),
            selection_feedback: clamp_weight(self.selection_feedback, Self::MEMORY_BOUNDS),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RankerFeatures {
    pub rime_rank: f32,
    pub typed: f32,
    pub voice: f32,
    pub clipboard: f32,
    pub recency: f32,
    pub app_context: f32,
    pub selection_feedback: f32,
}

impl RankerFeatures {
    pub fn score(&self, weights: &RankerWeights) -> f32 {
        self.rime_rank * weights.rime_rank
            + self.typed * weights.typed
            + self.voice * weights.voice
            + self.clipboard * weights.clipboard
            + self.recency * weights.recency
            + self.app_context * weights.app_context
            + self.selection_feedback * weights.selection_feedback
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RankingFeedback {
    pub composing: String,
    pub selected_text: String,
    pub displayed_candidates: Vec<String>,
    pub selected_index: usize,
}

impl RankingFeedback {
    pub fn new(
        composing: impl Into<String>,
        selected_text: impl Into<String>,
        displayed_candidates: Vec<String>,
        selected_index: usize,
    ) -> Self {
        Self {
            composing: composing.into(),
            selected_text: selected_text.into(),
            displayed_candidates,
            selected_index,
        }
    }

    fn from_candidates(
        composing: impl Into<String>,
        displayed_candidates: &[Candidate],
        selected_index: usize,
    ) -> Option<Self> {
        let selected = displayed_candidates.get(selected_index)?;
        Some(Self::new(
            composing,
            selected.text.clone(),
            displayed_candidates
                .iter()
                .map(|candidate| candidate.text.clone())
                .collect(),
            selected_index,
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryTerm {
    pub text: String,
    pub typed_count: u32,
    pub voice_count: u32,
    pub clipboard_count: u32,
    pub last_used_tick: u64,
}

impl MemoryTerm {
    fn new(text: String, tick: u64) -> Self {
        Self {
            text,
            typed_count: 0,
            voice_count: 0,
            clipboard_count: 0,
            last_used_tick: tick,
        }
    }

    fn bump(&mut self, source: &MemorySource, tick: u64) {
        match source {
            MemorySource::Typed => self.typed_count += 1,
            MemorySource::Voice => self.voice_count += 1,
            MemorySource::Clipboard => self.clipboard_count += 1,
        }
        self.last_used_tick = tick;
    }

    fn score(&self) -> i32 {
        let features = self.ranker_features(0.0, 0.0, 0.0, self.last_used_tick);
        features.score(&RankerWeights::default()).round() as i32
    }

    fn ranker_features(
        &self,
        rime_rank: f32,
        selection_feedback: f32,
        app_context: f32,
        current_tick: u64,
    ) -> RankerFeatures {
        RankerFeatures {
            rime_rank,
            typed: self.typed_count as f32,
            voice: self.voice_count as f32,
            clipboard: self.clipboard_count as f32,
            recency: recency_feature(self.last_used_tick, current_tick),
            app_context,
            selection_feedback,
        }
    }

    fn source_count(&self, source: &MemorySource) -> u32 {
        match source {
            MemorySource::Typed => self.typed_count,
            MemorySource::Voice => self.voice_count,
            MemorySource::Clipboard => self.clipboard_count,
        }
    }

    fn source_score(&self, source: &MemorySource) -> i32 {
        (self.source_count(source) as i32 * 100) + self.recency_score()
    }

    fn recency_score(&self) -> i32 {
        self.last_used_tick.min(10) as i32
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppPolicy {
    sensitive_bundle_ids: Vec<String>,
    sensitive_context_terms: Vec<String>,
}

impl Default for AppPolicy {
    fn default() -> Self {
        Self {
            sensitive_bundle_ids: vec![
                "com.1password.1password".to_string(),
                "com.agilebits.onepassword7".to_string(),
                "com.apple.Safari.PrivateBrowsing".to_string(),
                "com.bitwarden.desktop".to_string(),
                "com.lastpass.LastPass".to_string(),
                "com.protonmail.protonmail".to_string(),
            ],
            sensitive_context_terms: vec![
                "private browsing".to_string(),
                "private window".to_string(),
                "incognito".to_string(),
                "隐私浏览".to_string(),
                "无痕".to_string(),
                "登录".to_string(),
                "登陆".to_string(),
                "登入".to_string(),
                "密码".to_string(),
                "账号".to_string(),
                "账户".to_string(),
                "验证码".to_string(),
                "身份验证".to_string(),
                "认证".to_string(),
                "password".to_string(),
                "login".to_string(),
                "log in".to_string(),
                "sign in".to_string(),
                "signin".to_string(),
                "sign-in".to_string(),
                "authentication".to_string(),
                "otp".to_string(),
                "2fa".to_string(),
                "银行".to_string(),
                "bank".to_string(),
                "医疗".to_string(),
                "medical".to_string(),
            ],
        }
    }
}

impl AppPolicy {
    pub fn with_sensitive_bundle_ids(sensitive_bundle_ids: Vec<String>) -> Self {
        Self {
            sensitive_bundle_ids,
            sensitive_context_terms: Self::default().sensitive_context_terms,
        }
    }

    pub fn excludes(&self, context: &AppContext) -> bool {
        let bundle_id = context.bundle_id.trim();
        if bundle_id.is_empty() || bundle_id == "unknown" {
            return true;
        }
        if self
            .sensitive_bundle_ids
            .iter()
            .any(|sensitive_bundle_id| sensitive_bundle_id == bundle_id)
        {
            return true;
        }
        for value in [
            context.window_title.as_deref(),
            context.document_id.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let value = value.to_lowercase();
            if self
                .sensitive_context_terms
                .iter()
                .any(|term| value.contains(term))
            {
                return true;
            }
        }
        false
    }
}

fn default_ranking_context() -> AppContext {
    AppContext::new("dev.inputia.ranker")
}

#[derive(Clone, Debug, PartialEq)]
pub struct LocalMemory {
    policy: AppPolicy,
    tick: u64,
    terms: Vec<MemoryTerm>,
    weights: RankerWeights,
    preferences: Vec<RankerPreference>,
}

#[derive(Clone, Debug, PartialEq)]
struct RankerPreference {
    text: String,
    app_bundle_id: String,
    positive_count: f32,
    negative_count: f32,
    last_feedback_tick: u64,
}

impl RankerPreference {
    fn score(&self, current_tick: u64) -> f32 {
        let raw_score = self.positive_count - self.negative_count;
        let age = current_tick.saturating_sub(self.last_feedback_tick) as f32;
        let decay = 1.0 / (1.0 + age / 64.0);
        (raw_score * decay).clamp(-4.0, 4.0)
    }
}

impl Default for LocalMemory {
    fn default() -> Self {
        Self::new(AppPolicy::default())
    }
}

impl LocalMemory {
    pub fn new(policy: AppPolicy) -> Self {
        Self {
            policy,
            tick: 0,
            terms: Vec::new(),
            weights: RankerWeights::default(),
            preferences: Vec::new(),
        }
    }

    pub fn learn(
        &mut self,
        source: MemorySource,
        text: impl Into<String>,
        context: &AppContext,
    ) -> LearningOutcome {
        if self.policy.excludes(context) {
            return LearningOutcome {
                decision: PrivacyDecision::Excluded,
                term: None,
            };
        }

        let text = normalize_term(text.into());
        if text.is_empty() {
            return LearningOutcome {
                decision: PrivacyDecision::Learn,
                term: None,
            };
        }

        self.tick += 1;
        if let Some(term) = self.terms.iter_mut().find(|term| term.text == text) {
            term.bump(&source, self.tick);
        } else {
            let mut term = MemoryTerm::new(text.clone(), self.tick);
            term.bump(&source, self.tick);
            self.terms.push(term);
        }

        LearningOutcome {
            decision: PrivacyDecision::Learn,
            term: Some(text),
        }
    }

    pub fn rank_candidates(&self, candidates: Vec<Candidate>) -> Vec<Candidate> {
        self.rank_candidates_for_context(candidates, &default_ranking_context())
    }

    pub fn rank_candidates_for_context(
        &self,
        candidates: Vec<Candidate>,
        context: &AppContext,
    ) -> Vec<Candidate> {
        if self.policy.excludes(context) {
            return candidates;
        }

        let leading_engine_len = candidates
            .first()
            .map(|candidate| candidate.text.chars().count())
            .unwrap_or(0);
        let current_tick = self.current_tick();
        let weights = self.weights.clamped();
        let mut ranked = candidates
            .into_iter()
            .enumerate()
            .map(|(original_index, mut candidate)| {
                let rime_rank = candidate.base_score as f32;
                let selection_feedback =
                    self.selection_feedback_score(&candidate.text, current_tick);
                let app_context = self.app_context_score(&candidate.text, context, current_tick);
                let features = if let Some(term) =
                    self.terms.iter().find(|term| term.text == candidate.text)
                {
                    candidate.source = strongest_candidate_source(term);
                    term.ranker_features(rime_rank, selection_feedback, app_context, current_tick)
                } else {
                    RankerFeatures {
                        rime_rank,
                        selection_feedback,
                        app_context,
                        ..RankerFeatures::default()
                    }
                };
                let effective_score = features.score(&weights).round() as i32;
                candidate.memory_score += effective_score - candidate.base_score;
                let guarded_by_leading =
                    candidate_is_guarded_by_leading_phrase(&candidate, leading_engine_len);
                let ranking_score = candidate.final_score();
                (original_index, guarded_by_leading, ranking_score, candidate)
            })
            .collect::<Vec<_>>();

        ranked.sort_by(|left, right| {
            left.1
                .cmp(&right.1)
                .then_with(|| right.2.cmp(&left.2))
                .then_with(|| left.0.cmp(&right.0))
        });
        ranked
            .into_iter()
            .map(|(_, _, _, candidate)| candidate)
            .collect()
    }

    pub fn record_ranking_feedback(
        &mut self,
        feedback: RankingFeedback,
        context: &AppContext,
    ) -> PrivacyDecision {
        if self.policy.excludes(context) {
            return PrivacyDecision::Excluded;
        }

        if feedback.selected_text.trim().is_empty() {
            return PrivacyDecision::Learn;
        }

        self.tick += 1;
        let tick = self.tick;
        self.bump_preference(&feedback.selected_text, &context.bundle_id, 1.0, 0.0, tick);
        for candidate in feedback.displayed_candidates {
            if candidate == feedback.selected_text || candidate.trim().is_empty() {
                continue;
            }
            self.bump_preference(&candidate, &context.bundle_id, 0.0, 0.25, tick);
        }
        PrivacyDecision::Learn
    }

    pub fn completion_candidates(&self, prefix: &str, limit: usize) -> Vec<Candidate> {
        let mut terms = self
            .terms
            .iter()
            .filter(|term| term.text.starts_with(prefix))
            .cloned()
            .collect::<Vec<_>>();
        terms.sort_by(|left, right| right.score().cmp(&left.score()));
        terms
            .into_iter()
            .take(limit)
            .map(|term| {
                let score = term.score();
                let mut candidate = Candidate::new(format!("memory:{}", term.text), term.text);
                candidate.source = CandidateSource::Memory;
                candidate.memory_score = score;
                candidate
            })
            .collect()
    }

    pub fn english_completion_candidates(&self, prefix: &str, limit: usize) -> Vec<Candidate> {
        let normalized_prefix = prefix.trim();
        if normalized_prefix.len() < 2 || !normalized_prefix.is_ascii() {
            return Vec::new();
        }
        let lower_prefix = normalized_prefix.to_ascii_lowercase();
        let mut terms = self
            .terms
            .iter()
            .filter(|term| is_ascii_word(&term.text))
            .filter(|term| term.text.len() > normalized_prefix.len())
            .filter(|term| term.text.to_ascii_lowercase().starts_with(&lower_prefix))
            .cloned()
            .collect::<Vec<_>>();
        terms.sort_by(|left, right| {
            right
                .score()
                .cmp(&left.score())
                .then_with(|| left.text.cmp(&right.text))
        });
        terms
            .into_iter()
            .take(limit)
            .map(|term| {
                let score = term.score();
                let mut candidate =
                    Candidate::new(format!("english_completion:{}", term.text), term.text);
                candidate.source = CandidateSource::EnglishCompletion;
                candidate.memory_score = score;
                candidate
            })
            .collect()
    }

    pub fn source_candidates(&self, source: MemorySource, limit: usize) -> Vec<Candidate> {
        let mut terms = self
            .terms
            .iter()
            .filter(|term| term.source_count(&source) > 0)
            .cloned()
            .collect::<Vec<_>>();
        terms.sort_by(|left, right| {
            right
                .source_score(&source)
                .cmp(&left.source_score(&source))
                .then_with(|| right.last_used_tick.cmp(&left.last_used_tick))
                .then_with(|| left.text.cmp(&right.text))
        });

        terms
            .into_iter()
            .take(limit)
            .map(|term| {
                let source_score = term.source_score(&source);
                let mut candidate =
                    Candidate::new(format!("{}:{}", source.as_str(), term.text), term.text);
                candidate.source = match &source {
                    MemorySource::Typed => CandidateSource::Memory,
                    MemorySource::Voice => CandidateSource::Voice,
                    MemorySource::Clipboard => CandidateSource::Clipboard,
                };
                candidate.memory_score = source_score;
                candidate
            })
            .collect()
    }

    pub fn clipboard_candidates(&self, limit: usize) -> Vec<Candidate> {
        self.source_candidates(MemorySource::Clipboard, limit)
    }

    pub fn voice_hotwords(&self, limit: usize) -> Vec<String> {
        let mut terms = self.terms.clone();
        terms.sort_by(|left, right| right.score().cmp(&left.score()));
        terms
            .into_iter()
            .filter(|term| term.typed_count > 0 || term.voice_count > 0)
            .take(limit)
            .map(|term| term.text)
            .collect()
    }

    fn current_tick(&self) -> u64 {
        self.tick
            .max(
                self.terms
                    .iter()
                    .map(|term| term.last_used_tick)
                    .max()
                    .unwrap_or(0),
            )
            .max(
                self.preferences
                    .iter()
                    .map(|preference| preference.last_feedback_tick)
                    .max()
                    .unwrap_or(0),
            )
    }

    fn selection_feedback_score(&self, text: &str, current_tick: u64) -> f32 {
        self.preferences
            .iter()
            .filter(|preference| preference.text == text)
            .map(|preference| preference.score(current_tick))
            .sum::<f32>()
            .clamp(-4.0, 4.0)
    }

    fn app_context_score(&self, text: &str, context: &AppContext, current_tick: u64) -> f32 {
        self.preferences
            .iter()
            .filter(|preference| {
                preference.text == text && preference.app_bundle_id == context.bundle_id
            })
            .map(|preference| preference.score(current_tick))
            .sum::<f32>()
            .clamp(-4.0, 4.0)
    }

    fn bump_preference(
        &mut self,
        text: &str,
        app_bundle_id: &str,
        positive_delta: f32,
        negative_delta: f32,
        tick: u64,
    ) {
        if let Some(preference) = self
            .preferences
            .iter_mut()
            .find(|preference| preference.text == text && preference.app_bundle_id == app_bundle_id)
        {
            preference.positive_count =
                (preference.positive_count + positive_delta).clamp(0.0, 64.0);
            preference.negative_count =
                (preference.negative_count + negative_delta).clamp(0.0, 64.0);
            preference.last_feedback_tick = tick;
            return;
        }

        self.preferences.push(RankerPreference {
            text: text.to_string(),
            app_bundle_id: app_bundle_id.to_string(),
            positive_count: positive_delta,
            negative_count: negative_delta,
            last_feedback_tick: tick,
        });
    }
}

fn candidate_is_guarded_by_leading_phrase(
    candidate: &Candidate,
    leading_engine_len: usize,
) -> bool {
    let candidate_len = candidate.text.chars().count();
    leading_engine_len > 1 && candidate_len < leading_engine_len
}

fn clamp_weight(value: f32, bounds: (f32, f32)) -> f32 {
    if value.is_finite() {
        value.clamp(bounds.0, bounds.1)
    } else {
        bounds.0
    }
}

fn recency_feature(last_used_tick: u64, current_tick: u64) -> f32 {
    if last_used_tick == 0 {
        return 0.0;
    }
    let age = current_tick.saturating_sub(last_used_tick) as f32;
    10.0 / (1.0 + age / 16.0)
}

#[cfg(feature = "sqlite-memory")]
pub struct SqliteMemory {
    conn: rusqlite::Connection,
    policy: AppPolicy,
}

#[cfg(feature = "sqlite-memory")]
impl SqliteMemory {
    pub fn open(path: impl AsRef<Path>, policy: AppPolicy) -> rusqlite::Result<Self> {
        let conn = rusqlite::Connection::open(path)?;
        let memory = Self { conn, policy };
        memory.migrate()?;
        Ok(memory)
    }

    pub fn open_in_memory(policy: AppPolicy) -> rusqlite::Result<Self> {
        let conn = rusqlite::Connection::open_in_memory()?;
        let memory = Self { conn, policy };
        memory.migrate()?;
        Ok(memory)
    }

    pub fn learn(
        &mut self,
        source: MemorySource,
        text: impl Into<String>,
        context: &AppContext,
    ) -> rusqlite::Result<LearningOutcome> {
        if self.policy.excludes(context) {
            self.insert_event(&source, None, context, &PrivacyDecision::Excluded)?;
            return Ok(LearningOutcome {
                decision: PrivacyDecision::Excluded,
                term: None,
            });
        }

        let text = normalize_term(text.into());
        if text.is_empty() {
            self.insert_event(&source, None, context, &PrivacyDecision::Learn)?;
            return Ok(LearningOutcome {
                decision: PrivacyDecision::Learn,
                term: None,
            });
        }

        let tick = self.next_tick()?;
        let (typed, voice, clipboard) = source_counts(&source);
        self.conn.execute(
            "INSERT INTO inputia_terms (
                text, typed_count, voice_count, clipboard_count, last_used_tick
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(text) DO UPDATE SET
                typed_count = typed_count + excluded.typed_count,
                voice_count = voice_count + excluded.voice_count,
                clipboard_count = clipboard_count + excluded.clipboard_count,
                last_used_tick = excluded.last_used_tick",
            rusqlite::params![text, typed, voice, clipboard, tick],
        )?;
        self.insert_event(&source, Some(&text), context, &PrivacyDecision::Learn)?;
        Ok(LearningOutcome {
            decision: PrivacyDecision::Learn,
            term: Some(text),
        })
    }

    pub fn import_handy_history(
        &mut self,
        history_db_path: impl AsRef<Path>,
        context: &AppContext,
        limit: usize,
    ) -> rusqlite::Result<usize> {
        let source = open_read_only(history_db_path)?;
        let mut statement = source.prepare(
            "SELECT text FROM (
                SELECT COALESCE(
                    NULLIF(TRIM(post_processed_text), ''),
                    NULLIF(TRIM(transcription_text), '')
                ) AS text
                FROM transcription_history
                ORDER BY timestamp ASC
            )
            WHERE text IS NOT NULL
            LIMIT ?1",
        )?;
        let rows = statement.query_map([limit as i64], |row| row.get::<_, String>(0))?;
        let mut imported = 0;
        for row in rows {
            if self
                .learn(MemorySource::Voice, row?, context)?
                .term
                .is_some()
            {
                imported += 1;
            }
        }
        Ok(imported)
    }

    pub fn import_handy_clipboard(
        &mut self,
        clipboard_db_path: impl AsRef<Path>,
        fallback_context: &AppContext,
        limit: usize,
    ) -> rusqlite::Result<usize> {
        let source = open_read_only(clipboard_db_path)?;
        let mut statement = source.prepare(
            "SELECT text, source_app FROM (
                SELECT
                    COALESCE(
                        NULLIF(TRIM(full_text), ''),
                        NULLIF(TRIM(content_preview), '')
                    ) AS text,
                    source_app
                FROM clipboard_history
                WHERE LOWER(content_type) = 'text'
                ORDER BY created_at ASC
            )
            WHERE text IS NOT NULL
            LIMIT ?1",
        )?;
        let rows = statement.query_map([limit as i64], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;

        let mut imported = 0;
        for row in rows {
            let (text, source_app) = row?;
            let row_context = source_app
                .filter(|source_app| !source_app.trim().is_empty())
                .map(AppContext::new)
                .unwrap_or_else(|| fallback_context.clone());
            if self
                .learn(MemorySource::Clipboard, text, &row_context)?
                .term
                .is_some()
            {
                imported += 1;
            }
        }
        Ok(imported)
    }

    pub fn rank_candidates(&self, candidates: Vec<Candidate>) -> rusqlite::Result<Vec<Candidate>> {
        self.rank_candidates_for_context(candidates, &default_ranking_context())
    }

    pub fn rank_candidates_for_context(
        &self,
        candidates: Vec<Candidate>,
        context: &AppContext,
    ) -> rusqlite::Result<Vec<Candidate>> {
        let memory = LocalMemory {
            policy: self.policy.clone(),
            tick: self.current_tick()? as u64,
            terms: self.load_terms()?,
            weights: self.ranker_weights()?,
            preferences: self.load_ranking_preferences()?,
        };
        Ok(memory.rank_candidates_for_context(candidates, context))
    }

    pub fn record_ranking_feedback(
        &mut self,
        feedback: RankingFeedback,
        context: &AppContext,
    ) -> rusqlite::Result<PrivacyDecision> {
        if self.policy.excludes(context) {
            return Ok(PrivacyDecision::Excluded);
        }

        let selected_text = normalize_term(feedback.selected_text);
        if selected_text.is_empty() {
            return Ok(PrivacyDecision::Learn);
        }

        let tick = self.next_tick()?;
        self.bump_sqlite_preference(&selected_text, &context.bundle_id, 1.0, 0.0, tick)?;
        for candidate in feedback.displayed_candidates {
            let candidate = normalize_term(candidate);
            if candidate.is_empty() || candidate == selected_text {
                continue;
            }
            self.bump_sqlite_preference(&candidate, &context.bundle_id, 0.0, 0.25, tick)?;
        }
        Ok(PrivacyDecision::Learn)
    }

    pub fn ranker_weights(&self) -> rusqlite::Result<RankerWeights> {
        let mut weights = RankerWeights::default();
        let mut statement = self
            .conn
            .prepare("SELECT feature, weight FROM inputia_ranker_weights")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)? as f32))
        })?;
        for row in rows {
            let (feature, weight) = row?;
            match feature.as_str() {
                "rime_rank" => weights.rime_rank = weight,
                "typed" => weights.typed = weight,
                "voice" => weights.voice = weight,
                "clipboard" => weights.clipboard = weight,
                "recency" => weights.recency = weight,
                "app_context" => weights.app_context = weight,
                "selection_feedback" => weights.selection_feedback = weight,
                _ => {}
            }
        }
        Ok(weights.clamped())
    }

    pub fn save_ranker_weights(&self, weights: RankerWeights) -> rusqlite::Result<()> {
        let weights = weights.clamped();
        let tick = self.next_tick()?;
        for (feature, weight) in ranker_weight_rows(weights) {
            self.conn.execute(
                "INSERT INTO inputia_ranker_weights (feature, weight, updated_tick)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(feature) DO UPDATE SET
                    weight = excluded.weight,
                    updated_tick = excluded.updated_tick",
                rusqlite::params![feature, weight as f64, tick],
            )?;
        }
        Ok(())
    }

    pub fn ranking_feedback_count(&self) -> rusqlite::Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM inputia_ranking_feedback", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| count as usize)
    }

    pub fn completion_candidates(
        &self,
        prefix: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<Candidate>> {
        let memory = LocalMemory {
            policy: self.policy.clone(),
            tick: self.current_tick()? as u64,
            terms: self.load_terms()?,
            weights: self.ranker_weights()?,
            preferences: self.load_ranking_preferences()?,
        };
        Ok(memory.completion_candidates(prefix, limit))
    }

    pub fn english_completion_candidates(
        &self,
        prefix: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<Candidate>> {
        let memory = LocalMemory {
            policy: self.policy.clone(),
            tick: self.current_tick()? as u64,
            terms: self.load_terms()?,
            weights: self.ranker_weights()?,
            preferences: self.load_ranking_preferences()?,
        };
        Ok(memory.english_completion_candidates(prefix, limit))
    }

    pub fn clipboard_candidates(&self, limit: usize) -> rusqlite::Result<Vec<Candidate>> {
        let memory = LocalMemory {
            policy: self.policy.clone(),
            tick: self.current_tick()? as u64,
            terms: self.load_terms()?,
            weights: self.ranker_weights()?,
            preferences: self.load_ranking_preferences()?,
        };
        Ok(memory.clipboard_candidates(limit))
    }

    pub fn voice_hotwords(&self, limit: usize) -> rusqlite::Result<Vec<String>> {
        let memory = LocalMemory {
            policy: self.policy.clone(),
            tick: self.current_tick()? as u64,
            terms: self.load_terms()?,
            weights: self.ranker_weights()?,
            preferences: self.load_ranking_preferences()?,
        };
        Ok(memory.voice_hotwords(limit))
    }

    pub fn term_count(&self) -> rusqlite::Result<usize> {
        self.conn
            .query_row("SELECT COUNT(*) FROM inputia_terms", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| count as usize)
    }

    pub fn event_count(&self, decision: PrivacyDecision) -> rusqlite::Result<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM inputia_events WHERE privacy_decision = ?1",
                [decision.as_str()],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count as usize)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        self.conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS inputia_terms (
                text TEXT PRIMARY KEY,
                typed_count INTEGER NOT NULL DEFAULT 0,
                voice_count INTEGER NOT NULL DEFAULT 0,
                clipboard_count INTEGER NOT NULL DEFAULT 0,
                last_used_tick INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS inputia_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                source TEXT NOT NULL,
                text TEXT,
                app_bundle_id TEXT NOT NULL,
                privacy_decision TEXT NOT NULL,
                created_tick INTEGER NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_inputia_events_source
                ON inputia_events(source, created_tick);
            CREATE INDEX IF NOT EXISTS idx_inputia_terms_score
                ON inputia_terms(last_used_tick DESC);

            CREATE TABLE IF NOT EXISTS inputia_ranker_weights (
                feature TEXT PRIMARY KEY,
                weight REAL NOT NULL,
                updated_tick INTEGER NOT NULL DEFAULT 0
            );

            CREATE TABLE IF NOT EXISTS inputia_ranking_feedback (
                text TEXT NOT NULL,
                app_bundle_id TEXT NOT NULL,
                positive_count REAL NOT NULL DEFAULT 0,
                negative_count REAL NOT NULL DEFAULT 0,
                last_feedback_tick INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY(text, app_bundle_id)
            );

            CREATE INDEX IF NOT EXISTS idx_inputia_ranking_feedback_text
                ON inputia_ranking_feedback(text);
            CREATE INDEX IF NOT EXISTS idx_inputia_ranking_feedback_app
                ON inputia_ranking_feedback(app_bundle_id, last_feedback_tick DESC);",
        )?;
        self.ensure_default_ranker_weights()
    }

    fn next_tick(&self) -> rusqlite::Result<i64> {
        Ok(self.current_tick()? + 1)
    }

    fn current_tick(&self) -> rusqlite::Result<i64> {
        self.conn.query_row(
            "SELECT COALESCE(MAX(tick), 0) FROM (
                SELECT MAX(last_used_tick) AS tick FROM inputia_terms
                UNION ALL
                SELECT MAX(created_tick) AS tick FROM inputia_events
                UNION ALL
                SELECT MAX(updated_tick) AS tick FROM inputia_ranker_weights
                UNION ALL
                SELECT MAX(last_feedback_tick) AS tick FROM inputia_ranking_feedback
            )",
            [],
            |row| row.get(0),
        )
    }

    fn ensure_default_ranker_weights(&self) -> rusqlite::Result<()> {
        for (feature, weight) in ranker_weight_rows(RankerWeights::default()) {
            self.conn.execute(
                "INSERT OR IGNORE INTO inputia_ranker_weights (feature, weight, updated_tick)
                 VALUES (?1, ?2, 0)",
                rusqlite::params![feature, weight as f64],
            )?;
        }
        Ok(())
    }

    fn bump_sqlite_preference(
        &self,
        text: &str,
        app_bundle_id: &str,
        positive_delta: f32,
        negative_delta: f32,
        tick: i64,
    ) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO inputia_ranking_feedback (
                text, app_bundle_id, positive_count, negative_count, last_feedback_tick
            ) VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(text, app_bundle_id) DO UPDATE SET
                positive_count = MIN(64.0, MAX(0.0, positive_count + excluded.positive_count)),
                negative_count = MIN(64.0, MAX(0.0, negative_count + excluded.negative_count)),
                last_feedback_tick = excluded.last_feedback_tick",
            rusqlite::params![
                text,
                app_bundle_id,
                positive_delta as f64,
                negative_delta as f64,
                tick
            ],
        )?;
        Ok(())
    }

    fn insert_event(
        &self,
        source: &MemorySource,
        text: Option<&str>,
        context: &AppContext,
        decision: &PrivacyDecision,
    ) -> rusqlite::Result<()> {
        let tick = self.next_tick()?;
        self.conn.execute(
            "INSERT INTO inputia_events (
                source, text, app_bundle_id, privacy_decision, created_tick
            ) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                source.as_str(),
                text,
                &context.bundle_id,
                decision.as_str(),
                tick
            ],
        )?;
        Ok(())
    }

    fn load_terms(&self) -> rusqlite::Result<Vec<MemoryTerm>> {
        let mut statement = self.conn.prepare(
            "SELECT text, typed_count, voice_count, clipboard_count, last_used_tick
             FROM inputia_terms",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(MemoryTerm {
                text: row.get(0)?,
                typed_count: row.get::<_, i64>(1)? as u32,
                voice_count: row.get::<_, i64>(2)? as u32,
                clipboard_count: row.get::<_, i64>(3)? as u32,
                last_used_tick: row.get::<_, i64>(4)? as u64,
            })
        })?;
        rows.collect()
    }

    fn load_ranking_preferences(&self) -> rusqlite::Result<Vec<RankerPreference>> {
        let mut statement = self.conn.prepare(
            "SELECT text, app_bundle_id, positive_count, negative_count, last_feedback_tick
             FROM inputia_ranking_feedback",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(RankerPreference {
                text: row.get(0)?,
                app_bundle_id: row.get(1)?,
                positive_count: row.get::<_, f64>(2)? as f32,
                negative_count: row.get::<_, f64>(3)? as f32,
                last_feedback_tick: row.get::<_, i64>(4)? as u64,
            })
        })?;
        rows.collect()
    }
}

pub struct InputiaCore<E> {
    mode: InputMode,
    composing: String,
    preedit: String,
    candidates: Vec<Candidate>,
    page: usize,
    settings: CoreSettings,
    engine: E,
    last_ranking_feedback: Option<RankingFeedback>,
}

impl<E: ChineseEngine> InputiaCore<E> {
    pub fn new(settings: CoreSettings, engine: E) -> Self {
        Self {
            mode: InputMode::English,
            composing: String::new(),
            preedit: String::new(),
            candidates: Vec::new(),
            page: 0,
            settings,
            engine,
            last_ranking_feedback: None,
        }
    }

    pub fn handle_key(&mut self, key: Key) -> InputOutcome {
        self.last_ranking_feedback = None;
        let mut consumed = true;
        let mut commit = None;

        match (&self.mode, key) {
            (_, Key::TogglePunctuation) => {
                self.clear_composition();
                self.settings.punctuation_preference = match self.settings.punctuation_preference {
                    PunctuationPreference::EnglishInChinese => {
                        PunctuationPreference::FollowInputMode
                    }
                    PunctuationPreference::FollowInputMode => {
                        PunctuationPreference::EnglishInChinese
                    }
                };
            }
            (_, Key::ToggleCharacterWidth) => {
                self.clear_composition();
                self.settings.character_width_preference =
                    match self.settings.character_width_preference {
                        CharacterWidthPreference::HalfWidth => CharacterWidthPreference::FullWidth,
                        CharacterWidthPreference::FullWidth => CharacterWidthPreference::HalfWidth,
                    };
            }
            (_, Key::ToggleInputMode) => {
                self.clear_composition();
                self.mode = match self.mode {
                    InputMode::English => InputMode::Chinese,
                    InputMode::Chinese => InputMode::English,
                };
            }
            (_, Key::Shift) if self.settings.shift_toggle_enabled => {
                self.clear_composition();
                self.mode = match self.mode {
                    InputMode::English => InputMode::Chinese,
                    InputMode::Chinese => InputMode::English,
                };
            }
            (InputMode::English, Key::Char(ch)) => {
                commit = Some(self.translate_character_width(ch).to_string());
            }
            (InputMode::English, _) => {
                consumed = false;
            }
            (InputMode::Chinese, Key::Char(ch)) if ch.is_ascii_alphabetic() => {
                self.composing.push(ch.to_ascii_lowercase());
                self.refresh_candidates();
            }
            (InputMode::Chinese, Key::Char('\'')) => {
                if self.composing.is_empty() || self.composing.ends_with('\'') {
                    consumed = false;
                } else {
                    self.composing.push('\'');
                    self.refresh_candidates();
                }
            }
            (InputMode::Chinese, Key::Char(ch)) if is_punctuation(ch) => {
                let punctuation = self.translate_punctuation(ch);
                commit = if self.composing.is_empty() {
                    Some(punctuation)
                } else {
                    let text = self.commit_visible_candidate_text_or_raw_composition(0);
                    Some(format!("{text}{punctuation}"))
                };
            }
            (InputMode::Chinese, Key::Backspace) => {
                if self.composing.pop().is_none() {
                    consumed = false;
                } else {
                    self.refresh_candidates();
                }
            }
            (InputMode::Chinese, Key::Escape) => {
                self.clear_composition();
            }
            (InputMode::Chinese, Key::Space) => {
                if self.composing.is_empty() {
                    consumed = false;
                } else {
                    commit = self.commit_candidate_or_raw_composition(0);
                }
            }
            (InputMode::Chinese, Key::Enter) => {
                if self.composing.is_empty() {
                    consumed = false;
                } else {
                    commit = Some(self.commit_raw_composition());
                }
            }
            (InputMode::Chinese, Key::Digit(digit)) if digit <= 9 => {
                if (1..=9).contains(&digit) && !self.visible_candidates().is_empty() {
                    commit = self.commit_candidate_on_page((digit - 1) as usize);
                } else if self.composing.is_empty() {
                    consumed = false;
                } else {
                    self.composing.push(char::from(b'0' + digit));
                    self.refresh_candidates();
                }
            }
            (InputMode::Chinese, Key::PageDown) => {
                if self.page + 1 < self.page_count() {
                    self.page += 1;
                }
            }
            (InputMode::Chinese, Key::PageUp) => {
                self.page = self.page.saturating_sub(1);
            }
            (InputMode::Chinese, _) => {
                consumed = false;
            }
        }

        self.outcome(consumed, commit)
    }

    pub fn set_mode(&mut self, mode: InputMode) -> InputOutcome {
        if self.mode != mode {
            self.clear_composition();
            self.mode = mode;
        }
        self.outcome(false, None)
    }

    pub fn snapshot(&self) -> InputSnapshot {
        InputSnapshot {
            mode: self.mode.clone(),
            composing: self.display_composing(),
            page: self.page,
            page_size: self.settings.candidate_page_size.max(1),
            visible_candidates: self.visible_candidates().to_vec(),
            panel_candidates: self.candidates.clone(),
        }
    }

    pub fn take_ranking_feedback(&mut self) -> Option<RankingFeedback> {
        self.last_ranking_feedback.take()
    }

    fn refresh_candidates(&mut self) {
        self.page = 0;
        if self.composing.is_empty() {
            self.candidates.clear();
            self.preedit.clear();
        } else {
            self.candidates = self.engine.candidates(&self.composing);
            self.preedit = self.composing.clone();
        }
    }

    fn clear_composition(&mut self) {
        self.composing.clear();
        self.preedit.clear();
        self.candidates.clear();
        self.page = 0;
    }

    fn commit_candidate_on_page(&mut self, page_index: usize) -> Option<String> {
        let candidate = self.visible_candidates().get(page_index)?.clone();
        self.remember_ranking_feedback(page_index);
        if let Some(commit) =
            self.engine
                .select_candidate(&self.composing, &candidate, self.page, page_index)
        {
            return self.apply_candidate_commit(commit);
        }
        let candidate = candidate.text;
        self.clear_composition();
        Some(candidate)
    }

    fn commit_candidate_or_raw_composition(&mut self, page_index: usize) -> Option<String> {
        let Some(candidate) = self.visible_candidates().get(page_index).cloned() else {
            return Some(self.commit_raw_composition());
        };
        self.remember_ranking_feedback(page_index);
        if let Some(commit) =
            self.engine
                .select_candidate(&self.composing, &candidate, self.page, page_index)
        {
            return self.apply_candidate_commit(commit);
        }
        let commit = candidate.text;
        self.clear_composition();
        Some(commit)
    }

    fn commit_visible_candidate_text_or_raw_composition(&mut self, page_index: usize) -> String {
        let Some(candidate) = self.visible_candidates().get(page_index).cloned() else {
            return self.commit_raw_composition();
        };
        self.remember_ranking_feedback(page_index);
        self.clear_composition();
        candidate.text
    }

    fn remember_ranking_feedback(&mut self, page_index: usize) {
        self.last_ranking_feedback = RankingFeedback::from_candidates(
            self.composing.clone(),
            self.visible_candidates(),
            page_index,
        );
    }

    fn apply_candidate_commit(&mut self, commit: CandidateCommit) -> Option<String> {
        self.composing = commit.remaining_composing;
        self.preedit = if commit.preedit.is_empty() {
            self.composing.clone()
        } else {
            commit.preedit
        };
        self.page = commit.page;
        self.candidates = commit.candidates;
        if self.composing.is_empty() {
            self.preedit.clear();
            self.candidates.clear();
            self.page = 0;
        }
        commit.text
    }

    fn display_composing(&self) -> String {
        if self.preedit.is_empty() {
            self.composing.clone()
        } else {
            self.preedit.clone()
        }
    }

    fn commit_raw_composition(&mut self) -> String {
        let commit = self.translate_ascii_string_width(&self.composing);
        self.clear_composition();
        commit
    }

    fn visible_candidates(&self) -> &[Candidate] {
        let page_size = self.settings.candidate_page_size.max(1);
        let start = self.page * page_size;
        let end = (start + page_size).min(self.candidates.len());
        if start >= self.candidates.len() {
            &[]
        } else {
            &self.candidates[start..end]
        }
    }

    fn page_count(&self) -> usize {
        let page_size = self.settings.candidate_page_size.max(1);
        self.candidates.len().div_ceil(page_size)
    }

    fn translate_punctuation(&self, ch: char) -> String {
        if self.settings.punctuation_preference == PunctuationPreference::EnglishInChinese {
            return ch.to_string();
        }

        match ch {
            ',' => "，".to_string(),
            '.' => "。".to_string(),
            '?' => "？".to_string(),
            '!' => "！".to_string(),
            ':' => "：".to_string(),
            ';' => "；".to_string(),
            _ => ch.to_string(),
        }
    }

    fn translate_ascii_string_width(&self, text: &str) -> String {
        text.chars()
            .map(|ch| self.translate_character_width(ch))
            .collect()
    }

    fn translate_character_width(&self, ch: char) -> char {
        if self.settings.character_width_preference == CharacterWidthPreference::HalfWidth {
            return ch;
        }
        full_width_ascii(ch).unwrap_or(ch)
    }

    fn outcome(&self, consumed: bool, commit: Option<String>) -> InputOutcome {
        InputOutcome {
            consumed,
            commit,
            snapshot: self.snapshot(),
        }
    }
}

fn is_punctuation(ch: char) -> bool {
    matches!(ch, ',' | '.' | '?' | '!' | ':' | ';')
}

fn full_width_ascii(ch: char) -> Option<char> {
    match ch {
        ' ' => Some('\u{3000}'),
        '!'..='~' => char::from_u32(ch as u32 - 0x21 + 0xff01),
        _ => None,
    }
}

fn is_ascii_word(text: &str) -> bool {
    text.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
}

fn normalize_term(text: String) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn strongest_candidate_source(term: &MemoryTerm) -> CandidateSource {
    let mut source = CandidateSource::Memory;
    let mut score = 0;
    if term.clipboard_count > score {
        source = CandidateSource::Clipboard;
        score = term.clipboard_count;
    }
    if term.voice_count > score {
        source = CandidateSource::Voice;
        score = term.voice_count;
    }
    if term.typed_count > score {
        source = CandidateSource::Memory;
    }
    source
}

#[cfg(feature = "sqlite-memory")]
fn source_counts(source: &MemorySource) -> (i64, i64, i64) {
    match source {
        MemorySource::Typed => (1, 0, 0),
        MemorySource::Voice => (0, 1, 0),
        MemorySource::Clipboard => (0, 0, 1),
    }
}

#[cfg(feature = "sqlite-memory")]
fn ranker_weight_rows(weights: RankerWeights) -> [(&'static str, f32); 7] {
    [
        ("rime_rank", weights.rime_rank),
        ("typed", weights.typed),
        ("voice", weights.voice),
        ("clipboard", weights.clipboard),
        ("recency", weights.recency),
        ("app_context", weights.app_context),
        ("selection_feedback", weights.selection_feedback),
    ]
}

#[cfg(feature = "sqlite-memory")]
fn open_read_only(path: impl AsRef<Path>) -> rusqlite::Result<rusqlite::Connection> {
    rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct StubEngine;

    #[derive(Default)]
    struct ToneEngine;

    #[derive(Default)]
    struct SegmentingEngine;

    impl ChineseEngine for StubEngine {
        fn candidates(&self, composing: &str) -> Vec<Candidate> {
            match composing {
                "ni" => ["你", "拟", "尼", "泥", "呢", "逆", "腻", "妮", "霓"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, text)| Candidate::new(format!("ni-{index}"), text))
                    .collect(),
                "zhongguo" => ["中国", "种过", "种果", "忠果", "中古"]
                    .into_iter()
                    .enumerate()
                    .map(|(index, text)| Candidate::new(format!("zhongguo-{index}"), text))
                    .collect(),
                _ => Vec::new(),
            }
        }
    }

    impl ChineseEngine for ToneEngine {
        fn candidates(&self, composing: &str) -> Vec<Candidate> {
            match composing {
                "ni3" => [Candidate::new("ni3-0", "你")].into_iter().collect(),
                _ => Vec::new(),
            }
        }
    }

    impl ChineseEngine for SegmentingEngine {
        fn candidates(&self, composing: &str) -> Vec<Candidate> {
            match composing {
                "nillem" => [
                    Candidate::new("segment:0", "你"),
                    Candidate::new("segment:1", "尼"),
                ]
                .into_iter()
                .collect(),
                "llem" => [Candidate::new("segment:remaining:0", "来了吗")]
                    .into_iter()
                    .collect(),
                _ => Vec::new(),
            }
        }

        fn select_candidate(
            &self,
            composing: &str,
            candidate: &Candidate,
            _page: usize,
            _page_index: usize,
        ) -> Option<CandidateCommit> {
            match (composing, candidate.text.as_str()) {
                ("nillem", "你") => Some(CandidateCommit::new(
                    "你",
                    "llem",
                    0,
                    self.candidates("llem"),
                )),
                ("llem", "来了吗") => Some(CandidateCommit::new("来了吗", "", 0, Vec::new())),
                _ => None,
            }
        }
    }

    fn core() -> InputiaCore<StubEngine> {
        InputiaCore::new(CoreSettings::default(), StubEngine)
    }

    fn feed<E: ChineseEngine>(core: &mut InputiaCore<E>, text: &str) -> InputOutcome {
        let mut outcome = core.snapshot_outcome();
        for ch in text.chars() {
            outcome = core.handle_key(Key::Char(ch));
        }
        outcome
    }

    trait SnapshotOutcome {
        fn snapshot_outcome(&self) -> InputOutcome;
    }

    impl<E: ChineseEngine> SnapshotOutcome for InputiaCore<E> {
        fn snapshot_outcome(&self) -> InputOutcome {
            self.outcome(false, None)
        }
    }

    #[test]
    fn english_mode_commits_characters_directly() {
        let mut core = core();
        let outcome = core.handle_key(Key::Char('a'));
        assert!(outcome.consumed);
        assert_eq!(outcome.commit.as_deref(), Some("a"));
        assert_eq!(outcome.snapshot.mode, InputMode::English);
    }

    #[test]
    fn full_width_mode_translates_english_direct_input() {
        let mut core = InputiaCore::new(
            CoreSettings {
                character_width_preference: CharacterWidthPreference::FullWidth,
                ..CoreSettings::default()
            },
            StubEngine,
        );

        assert_eq!(
            core.handle_key(Key::Char('A')).commit.as_deref(),
            Some("Ａ")
        );
        assert_eq!(
            core.handle_key(Key::Char('1')).commit.as_deref(),
            Some("１")
        );
        assert_eq!(
            core.handle_key(Key::Char('!')).commit.as_deref(),
            Some("！")
        );
        assert_eq!(
            core.handle_key(Key::Char(' ')).commit.as_deref(),
            Some("　")
        );
    }

    #[test]
    fn shift_toggles_between_english_and_chinese() {
        let mut core = core();
        assert_eq!(
            core.handle_key(Key::Shift).snapshot.mode,
            InputMode::Chinese
        );
        assert_eq!(
            core.handle_key(Key::Shift).snapshot.mode,
            InputMode::English
        );
    }

    #[test]
    fn explicit_mode_set_does_not_depend_on_shift_toggle_state() {
        let mut core = core();
        assert_eq!(
            core.set_mode(InputMode::Chinese).snapshot.mode,
            InputMode::Chinese
        );
        assert_eq!(core.handle_key(Key::Char('z')).snapshot.composing, "z");
        assert_eq!(
            core.set_mode(InputMode::English).snapshot.mode,
            InputMode::English
        );

        let mut no_shift_core = InputiaCore::new(
            CoreSettings {
                shift_toggle_enabled: false,
                ..CoreSettings::default()
            },
            StubEngine,
        );
        assert_eq!(
            no_shift_core.set_mode(InputMode::Chinese).snapshot.mode,
            InputMode::Chinese
        );
    }

    #[test]
    fn explicit_input_mode_toggle_does_not_depend_on_shift_toggle_state() {
        let mut core = InputiaCore::new(
            CoreSettings {
                shift_toggle_enabled: false,
                ..CoreSettings::default()
            },
            StubEngine,
        );

        assert_eq!(
            core.handle_key(Key::ToggleInputMode).snapshot.mode,
            InputMode::Chinese
        );
        assert_eq!(
            core.handle_key(Key::ToggleInputMode).snapshot.mode,
            InputMode::English
        );
    }

    #[test]
    fn chinese_mode_builds_composition_and_candidates() {
        let mut core = core();
        core.handle_key(Key::Shift);
        let outcome = feed(&mut core, "ni");
        assert_eq!(outcome.snapshot.composing, "ni");
        assert_eq!(outcome.snapshot.visible_candidates[0].text, "你");
        assert_eq!(outcome.snapshot.visible_candidates.len(), 7);
    }

    #[test]
    fn apostrophe_separates_chinese_composition() {
        let mut core = core();
        core.handle_key(Key::Shift);

        let outcome = feed(&mut core, "ni'hao");

        assert!(outcome.consumed);
        assert_eq!(outcome.snapshot.composing, "ni'hao");
    }

    #[test]
    fn backspace_updates_composition() {
        let mut core = core();
        core.handle_key(Key::Shift);
        feed(&mut core, "ni");
        let outcome = core.handle_key(Key::Backspace);
        assert_eq!(outcome.snapshot.composing, "n");
        assert!(outcome.snapshot.visible_candidates.is_empty());
    }

    #[test]
    fn escape_clears_composition() {
        let mut core = core();
        core.handle_key(Key::Shift);
        feed(&mut core, "ni");
        let outcome = core.handle_key(Key::Escape);
        assert_eq!(outcome.snapshot.composing, "");
        assert!(outcome.snapshot.visible_candidates.is_empty());
    }

    #[test]
    fn space_commits_first_candidate() {
        let mut core = core();
        core.handle_key(Key::Shift);
        feed(&mut core, "ni");
        let outcome = core.handle_key(Key::Space);
        assert_eq!(outcome.commit.as_deref(), Some("你"));
        assert_eq!(outcome.snapshot.composing, "");
    }

    #[test]
    fn space_commits_selected_segment_and_keeps_remaining_composition() {
        let mut core = InputiaCore::new(CoreSettings::default(), SegmentingEngine);
        core.handle_key(Key::Shift);
        feed(&mut core, "nillem");

        let first = core.handle_key(Key::Space);

        assert_eq!(first.commit.as_deref(), Some("你"));
        assert_eq!(first.snapshot.composing, "llem");
        assert_eq!(first.snapshot.visible_candidates[0].text, "来了吗");

        let remaining = core.handle_key(Key::Space);
        assert_eq!(remaining.commit.as_deref(), Some("来了吗"));
        assert_eq!(remaining.snapshot.composing, "");
    }

    #[test]
    fn space_passes_through_without_composition() {
        let mut core = core();
        core.handle_key(Key::Shift);
        let outcome = core.handle_key(Key::Space);
        assert!(!outcome.consumed);
        assert_eq!(outcome.commit, None);
    }

    #[test]
    fn space_commits_raw_composition_when_engine_has_no_candidates() {
        let mut core = core();
        core.handle_key(Key::Shift);
        feed(&mut core, "xx");
        let outcome = core.handle_key(Key::Space);
        assert!(outcome.consumed);
        assert_eq!(outcome.commit.as_deref(), Some("xx"));
        assert_eq!(outcome.snapshot.composing, "");
    }

    #[test]
    fn enter_commits_raw_composition_in_chinese_mode() {
        let mut core = core();
        core.handle_key(Key::Shift);
        feed(&mut core, "ni");
        let outcome = core.handle_key(Key::Enter);
        assert!(outcome.consumed);
        assert_eq!(outcome.commit.as_deref(), Some("ni"));
        assert_eq!(outcome.snapshot.composing, "");
        assert!(outcome.snapshot.visible_candidates.is_empty());
    }

    #[test]
    fn full_width_mode_translates_raw_composition_but_not_chinese_candidates() {
        let mut core = InputiaCore::new(
            CoreSettings {
                character_width_preference: CharacterWidthPreference::FullWidth,
                ..CoreSettings::default()
            },
            StubEngine,
        );

        core.handle_key(Key::Shift);
        feed(&mut core, "ni");
        let raw = core.handle_key(Key::Enter);
        assert_eq!(raw.commit.as_deref(), Some("ｎｉ"));

        feed(&mut core, "ni");
        let candidate = core.handle_key(Key::Space);
        assert_eq!(candidate.commit.as_deref(), Some("你"));
    }

    #[test]
    fn enter_passes_through_without_composition() {
        let mut core = core();
        core.handle_key(Key::Shift);
        let outcome = core.handle_key(Key::Enter);
        assert!(!outcome.consumed);
        assert_eq!(outcome.commit, None);
    }

    #[test]
    fn digits_extend_composition_when_no_candidate_is_visible() {
        let mut core = InputiaCore::new(CoreSettings::default(), ToneEngine);
        core.handle_key(Key::Shift);
        feed(&mut core, "ni");
        let outcome = core.handle_key(Key::Digit(3));
        assert!(outcome.consumed);
        assert_eq!(outcome.snapshot.composing, "ni3");
        assert_eq!(outcome.snapshot.visible_candidates[0].text, "你");
    }

    #[test]
    fn digit_selects_candidate_on_current_page() {
        let mut core = core();
        core.handle_key(Key::Shift);
        feed(&mut core, "ni");
        let outcome = core.handle_key(Key::Digit(3));
        assert_eq!(outcome.commit.as_deref(), Some("尼"));
    }

    #[test]
    fn paging_changes_visible_candidates() {
        let mut core = core();
        core.handle_key(Key::Shift);
        feed(&mut core, "ni");
        let outcome = core.handle_key(Key::PageDown);
        assert_eq!(outcome.snapshot.page, 1);
        assert_eq!(outcome.snapshot.visible_candidates[0].text, "妮");
        let outcome = core.handle_key(Key::PageUp);
        assert_eq!(outcome.snapshot.page, 0);
        assert_eq!(outcome.snapshot.visible_candidates[0].text, "你");
    }

    #[test]
    fn snapshot_keeps_panel_candidates_beyond_current_page() {
        let mut core = core();
        core.handle_key(Key::Shift);
        let outcome = feed(&mut core, "ni");

        assert_eq!(outcome.snapshot.visible_candidates.len(), 7);
        assert_eq!(outcome.snapshot.panel_candidates.len(), 9);
        assert_eq!(outcome.snapshot.visible_candidates[0].text, "你");
        assert_eq!(outcome.snapshot.panel_candidates[8].text, "霓");
    }

    #[test]
    fn chinese_mode_can_force_english_punctuation() {
        let mut core = core();
        core.handle_key(Key::Shift);
        let outcome = core.handle_key(Key::Char(','));
        assert_eq!(outcome.commit.as_deref(), Some(","));
    }

    #[test]
    fn punctuation_commits_first_candidate_before_punctuation() {
        let mut core = core();
        core.handle_key(Key::Shift);
        feed(&mut core, "ni");

        let outcome = core.handle_key(Key::Char(','));

        assert!(outcome.consumed);
        assert_eq!(outcome.commit.as_deref(), Some("你,"));
        assert_eq!(outcome.snapshot.composing, "");
    }

    #[test]
    fn follow_mode_punctuation_commits_first_candidate_before_chinese_punctuation() {
        let mut core = InputiaCore::new(
            CoreSettings {
                punctuation_preference: PunctuationPreference::FollowInputMode,
                ..CoreSettings::default()
            },
            StubEngine,
        );
        core.handle_key(Key::Shift);
        feed(&mut core, "ni");

        let outcome = core.handle_key(Key::Char(','));

        assert!(outcome.consumed);
        assert_eq!(outcome.commit.as_deref(), Some("你，"));
        assert_eq!(outcome.snapshot.composing, "");
    }

    #[test]
    fn english_punctuation_in_chinese_mode_stays_half_width_even_in_full_width_mode() {
        let mut core = InputiaCore::new(
            CoreSettings {
                character_width_preference: CharacterWidthPreference::FullWidth,
                punctuation_preference: PunctuationPreference::EnglishInChinese,
                ..CoreSettings::default()
            },
            StubEngine,
        );
        core.handle_key(Key::Shift);

        let outcome = core.handle_key(Key::Char(','));

        assert_eq!(outcome.commit.as_deref(), Some(","));
    }

    #[test]
    fn chinese_punctuation_is_available_when_configured() {
        let mut core = InputiaCore::new(
            CoreSettings {
                punctuation_preference: PunctuationPreference::FollowInputMode,
                ..CoreSettings::default()
            },
            StubEngine,
        );
        core.handle_key(Key::Shift);
        let outcome = core.handle_key(Key::Char(','));
        assert_eq!(outcome.commit.as_deref(), Some("，"));
    }

    #[test]
    fn toggle_punctuation_switches_chinese_punctuation_runtime() {
        let mut core = core();
        core.handle_key(Key::Shift);

        let english = core.handle_key(Key::Char(','));
        assert_eq!(english.commit.as_deref(), Some(","));

        core.handle_key(Key::TogglePunctuation);
        let chinese = core.handle_key(Key::Char(','));
        assert_eq!(chinese.commit.as_deref(), Some("，"));

        core.handle_key(Key::TogglePunctuation);
        let english_again = core.handle_key(Key::Char(','));
        assert_eq!(english_again.commit.as_deref(), Some(","));
    }

    #[test]
    fn toggle_character_width_switches_direct_and_raw_input_runtime() {
        let mut core = core();

        let half_width = core.handle_key(Key::Char('A'));
        assert_eq!(half_width.commit.as_deref(), Some("A"));

        core.handle_key(Key::ToggleCharacterWidth);
        let full_width = core.handle_key(Key::Char('A'));
        assert_eq!(full_width.commit.as_deref(), Some("Ａ"));

        core.handle_key(Key::Shift);
        feed(&mut core, "ni");
        let raw_full_width = core.handle_key(Key::Enter);
        assert_eq!(raw_full_width.commit.as_deref(), Some("ｎｉ"));

        core.handle_key(Key::ToggleCharacterWidth);
        feed(&mut core, "ni");
        let raw_half_width = core.handle_key(Key::Enter);
        assert_eq!(raw_half_width.commit.as_deref(), Some("ni"));
    }

    #[test]
    fn runtime_toggles_clear_uncommitted_composition() {
        let mut core = core();
        core.handle_key(Key::Shift);
        feed(&mut core, "ni");

        let outcome = core.handle_key(Key::ToggleCharacterWidth);

        assert_eq!(outcome.snapshot.mode, InputMode::Chinese);
        assert_eq!(outcome.snapshot.composing, "");
        assert!(outcome.snapshot.visible_candidates.is_empty());
    }

    #[test]
    fn memory_reranks_engine_candidates_from_typed_history() {
        let context = AppContext::new("com.apple.TextEdit");
        let mut memory = LocalMemory::new(AppPolicy::default());
        memory.learn(MemorySource::Typed, "泥", &context);
        memory.learn(MemorySource::Typed, "泥", &context);

        let ranked = memory.rank_candidates(vec![
            Candidate::new("ni-0", "你"),
            Candidate::new("ni-1", "拟"),
            Candidate::new("ni-2", "泥"),
        ]);

        assert_eq!(ranked[0].text, "泥");
        assert!(ranked[0].memory_score > ranked[1].memory_score);
    }

    #[test]
    fn ranker_features_from_typed_voice_and_clipboard_affect_sorting() {
        let context = AppContext::new("com.apple.TextEdit");
        let mut memory = LocalMemory::new(AppPolicy::default());
        memory.learn(MemorySource::Typed, "打字候选", &context);
        for _ in 0..3 {
            memory.learn(MemorySource::Voice, "语音候选", &context);
        }
        for _ in 0..4 {
            memory.learn(MemorySource::Clipboard, "剪贴板候选", &context);
        }

        let ranked = memory.rank_candidates(vec![
            Candidate::new("engine-0", "普通候选"),
            Candidate::new("engine-1", "剪贴板候选"),
            Candidate::new("engine-2", "打字候选"),
            Candidate::new("engine-3", "语音候选"),
        ]);

        assert_eq!(ranked[0].text, "语音候选");
        assert_eq!(ranked[0].source, CandidateSource::Voice);
        assert!(ranked.iter().any(|candidate| {
            candidate.text == "打字候选" && candidate.source == CandidateSource::Memory
        }));
        assert!(ranked.iter().any(|candidate| {
            candidate.text == "剪贴板候选" && candidate.source == CandidateSource::Clipboard
        }));
    }

    #[test]
    fn memory_ranking_preserves_engine_order_without_memory_score() {
        let memory = LocalMemory::new(AppPolicy::default());

        let ranked = memory.rank_candidates(vec![
            Candidate::new("engine-0", "乙"),
            Candidate::new("engine-1", "甲"),
            Candidate::new("engine-2", "丁"),
        ]);

        assert_eq!(
            ranked
                .iter()
                .map(|candidate| candidate.text.as_str())
                .collect::<Vec<_>>(),
            vec!["乙", "甲", "丁"]
        );
    }

    #[test]
    fn memory_does_not_promote_single_character_over_long_engine_candidate() {
        let context = AppContext::new("com.apple.TextEdit");
        let mut memory = LocalMemory::new(AppPolicy::default());
        for _ in 0..20 {
            memory.learn(MemorySource::Typed, "你", &context);
        }

        let ranked = memory.rank_candidates(vec![
            Candidate::new("rime:0", "你来了吗"),
            Candidate::new("rime:1", "你"),
            Candidate::new("rime:2", "你来"),
        ]);

        assert_eq!(ranked[0].text, "你来了吗");
        assert!(ranked.iter().any(|candidate| candidate.text == "你"));
    }

    #[test]
    fn ranking_feedback_promotes_selected_candidate_next_time() {
        let context = AppContext::new("com.apple.TextEdit");
        let mut memory = LocalMemory::new(AppPolicy::default());

        let feedback = RankingFeedback::new(
            "ni",
            "拟",
            vec!["你".to_string(), "拟".to_string(), "尼".to_string()],
            1,
        );
        assert_eq!(
            memory.record_ranking_feedback(feedback, &context),
            PrivacyDecision::Learn
        );

        let ranked = memory.rank_candidates_for_context(
            vec![
                Candidate::new("ni-0", "你"),
                Candidate::new("ni-1", "拟"),
                Candidate::new("ni-2", "尼"),
            ],
            &context,
        );

        assert_eq!(ranked[0].text, "拟");
        assert!(ranked[0].memory_score > ranked[1].memory_score);
    }

    #[test]
    fn voice_and_clipboard_history_can_create_completion_candidates() {
        let context = AppContext::new("com.apple.TextEdit");
        let mut memory = LocalMemory::new(AppPolicy::default());
        memory.learn(MemorySource::Voice, "中国市场", &context);
        memory.learn(MemorySource::Clipboard, "中国市场报告", &context);

        let completions = memory.completion_candidates("中国", 5);
        assert_eq!(completions.len(), 2);
        assert_eq!(completions[0].source, CandidateSource::Memory);
        assert!(completions
            .iter()
            .any(|candidate| candidate.text == "中国市场"));
        assert!(completions
            .iter()
            .any(|candidate| candidate.text == "中国市场报告"));
    }

    #[test]
    fn english_completion_candidates_are_case_insensitive_and_memory_ranked() {
        let context = AppContext::new("com.apple.TextEdit");
        let mut memory = LocalMemory::new(AppPolicy::default());
        memory.learn(MemorySource::Typed, "Inputia", &context);
        memory.learn(MemorySource::Typed, "Inputia", &context);
        memory.learn(MemorySource::Clipboard, "input-layer", &context);
        memory.learn(MemorySource::Voice, "中国市场", &context);

        let completions = memory.english_completion_candidates("in", 5);

        assert_eq!(completions[0].text, "Inputia");
        assert_eq!(completions[0].source, CandidateSource::EnglishCompletion);
        assert!(completions
            .iter()
            .any(|candidate| candidate.text == "input-layer"));
        assert!(completions
            .iter()
            .all(|candidate| candidate.text.is_ascii()));
        assert!(memory
            .english_completion_candidates("Inputia", 5)
            .is_empty());
        assert!(memory.english_completion_candidates("i", 5).is_empty());
    }

    #[test]
    fn clipboard_recall_returns_only_clipboard_terms() {
        let context = AppContext::new("com.apple.TextEdit");
        let mut memory = LocalMemory::new(AppPolicy::default());
        memory.learn(MemorySource::Voice, "语音 热词", &context);
        memory.learn(MemorySource::Typed, "打字 记忆", &context);
        memory.learn(MemorySource::Clipboard, "剪贴板 常用语", &context);
        memory.learn(MemorySource::Clipboard, "剪贴板 常用语", &context);
        memory.learn(MemorySource::Clipboard, "剪贴板 临时句", &context);

        let recalled = memory.clipboard_candidates(10);

        assert_eq!(recalled[0].text, "剪贴板 常用语");
        assert_eq!(recalled[0].source, CandidateSource::Clipboard);
        assert!(recalled
            .iter()
            .any(|candidate| candidate.text == "剪贴板 临时句"));
        assert!(!recalled
            .iter()
            .any(|candidate| candidate.text == "语音 热词"));
        assert!(!recalled
            .iter()
            .any(|candidate| candidate.text == "打字 记忆"));
    }

    #[test]
    fn user_committed_terms_become_voice_hotwords() {
        let context = AppContext::new("com.apple.TextEdit");
        let mut memory = LocalMemory::new(AppPolicy::default());
        memory.learn(MemorySource::Typed, "Inputia", &context);

        assert_eq!(memory.voice_hotwords(3), vec!["Inputia"]);
    }

    #[test]
    fn sensitive_apps_do_not_learn() {
        let context = AppContext::new("com.1password.1password");
        let mut memory = LocalMemory::new(AppPolicy::default());
        let outcome = memory.learn(MemorySource::Typed, "secret phrase", &context);

        assert_eq!(outcome.decision, PrivacyDecision::Excluded);
        assert!(memory.completion_candidates("secret", 5).is_empty());
        assert!(memory.voice_hotwords(5).is_empty());
    }

    #[test]
    fn sensitive_window_contexts_do_not_learn() {
        let context = AppContext::new("com.apple.Safari")
            .with_window_title(Some("Private Browsing - Bank Login".to_string()));
        let mut memory = LocalMemory::new(AppPolicy::default());
        let outcome = memory.learn(MemorySource::Typed, "secret phrase", &context);

        assert_eq!(outcome.decision, PrivacyDecision::Excluded);
        assert!(memory.completion_candidates("secret", 5).is_empty());
        assert!(memory.voice_hotwords(5).is_empty());
    }

    #[test]
    fn unknown_contexts_do_not_learn() {
        let context = AppContext::new("unknown");
        let mut memory = LocalMemory::new(AppPolicy::default());
        let outcome = memory.learn(MemorySource::Typed, "secret phrase", &context);

        assert_eq!(outcome.decision, PrivacyDecision::Excluded);
        assert!(memory.completion_candidates("secret", 5).is_empty());
        assert!(memory.voice_hotwords(5).is_empty());
    }

    #[test]
    fn sensitive_contexts_do_not_record_ranking_feedback() {
        let context = AppContext::new("com.1password.1password");
        let mut memory = LocalMemory::new(AppPolicy::default());
        let feedback = RankingFeedback::new(
            "ni",
            "拟",
            vec!["你".to_string(), "拟".to_string(), "尼".to_string()],
            1,
        );

        assert_eq!(
            memory.record_ranking_feedback(feedback, &context),
            PrivacyDecision::Excluded
        );
        let ranked = memory.rank_candidates_for_context(
            vec![Candidate::new("ni-0", "你"), Candidate::new("ni-1", "拟")],
            &context,
        );
        assert_eq!(ranked[0].text, "你");
    }

    #[test]
    fn unknown_contexts_do_not_record_ranking_feedback() {
        let context = AppContext::new("unknown");
        let mut memory = LocalMemory::new(AppPolicy::default());
        let feedback = RankingFeedback::new(
            "ni",
            "拟",
            vec!["你".to_string(), "拟".to_string(), "尼".to_string()],
            1,
        );

        assert_eq!(
            memory.record_ranking_feedback(feedback, &context),
            PrivacyDecision::Excluded
        );
        let ranked = memory.rank_candidates_for_context(
            vec![Candidate::new("ni-0", "你"), Candidate::new("ni-1", "拟")],
            &context,
        );
        assert_eq!(ranked[0].text, "你");
    }

    #[test]
    fn core_records_last_displayed_candidate_feedback_on_selection() {
        let mut core = core();
        core.handle_key(Key::Shift);
        feed(&mut core, "ni");

        let outcome = core.handle_key(Key::Digit(2));
        let feedback = core.take_ranking_feedback().unwrap();

        assert_eq!(outcome.commit.as_deref(), Some("拟"));
        assert_eq!(feedback.composing, "ni");
        assert_eq!(feedback.selected_text, "拟");
        assert_eq!(feedback.selected_index, 1);
        assert_eq!(feedback.displayed_candidates[0], "你");
    }

    #[cfg(feature = "sqlite-memory")]
    #[test]
    fn sqlite_memory_persists_terms_and_reranks_candidates() {
        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("inputia_memory.db");
        let context = AppContext::new("com.apple.TextEdit");

        {
            let mut memory = SqliteMemory::open(&db_path, AppPolicy::default()).unwrap();
            memory.learn(MemorySource::Typed, "泥", &context).unwrap();
            memory.learn(MemorySource::Typed, "泥", &context).unwrap();
        }

        let memory = SqliteMemory::open(&db_path, AppPolicy::default()).unwrap();
        let ranked = memory
            .rank_candidates(vec![
                Candidate::new("ni-0", "你"),
                Candidate::new("ni-1", "拟"),
                Candidate::new("ni-2", "泥"),
            ])
            .unwrap();

        assert_eq!(ranked[0].text, "泥");
        assert!(ranked[0].memory_score > ranked[1].memory_score);
    }

    #[cfg(feature = "sqlite-memory")]
    #[test]
    fn sqlite_ranker_weights_persist() {
        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("inputia_memory.db");

        {
            let memory = SqliteMemory::open(&db_path, AppPolicy::default()).unwrap();
            let weights = RankerWeights {
                typed: 77.0,
                voice: 44.0,
                selection_feedback: 123.0,
                ..RankerWeights::default()
            };
            memory.save_ranker_weights(weights).unwrap();
        }

        let memory = SqliteMemory::open(&db_path, AppPolicy::default()).unwrap();
        let weights = memory.ranker_weights().unwrap();
        assert_eq!(weights.typed, 77.0);
        assert_eq!(weights.voice, 44.0);
        assert_eq!(weights.selection_feedback, 123.0);
    }

    #[cfg(feature = "sqlite-memory")]
    #[test]
    fn sqlite_ranking_feedback_persists_and_reranks_candidates() {
        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("inputia_memory.db");
        let context = AppContext::new("com.apple.TextEdit");

        {
            let mut memory = SqliteMemory::open(&db_path, AppPolicy::default()).unwrap();
            memory
                .record_ranking_feedback(
                    RankingFeedback::new(
                        "ni",
                        "拟",
                        vec!["你".to_string(), "拟".to_string(), "尼".to_string()],
                        1,
                    ),
                    &context,
                )
                .unwrap();
        }

        let memory = SqliteMemory::open(&db_path, AppPolicy::default()).unwrap();
        assert_eq!(memory.ranking_feedback_count().unwrap(), 3);
        let ranked = memory
            .rank_candidates_for_context(
                vec![
                    Candidate::new("ni-0", "你"),
                    Candidate::new("ni-1", "拟"),
                    Candidate::new("ni-2", "尼"),
                ],
                &context,
            )
            .unwrap();

        assert_eq!(ranked[0].text, "拟");
    }

    #[cfg(feature = "sqlite-memory")]
    #[test]
    fn sqlite_sensitive_contexts_do_not_record_ranking_feedback() {
        let context = AppContext::new("com.1password.1password");
        let mut memory = SqliteMemory::open_in_memory(AppPolicy::default()).unwrap();
        let decision = memory
            .record_ranking_feedback(
                RankingFeedback::new("ni", "拟", vec!["你".to_string(), "拟".to_string()], 1),
                &context,
            )
            .unwrap();

        assert_eq!(decision, PrivacyDecision::Excluded);
        assert_eq!(memory.ranking_feedback_count().unwrap(), 0);
    }

    #[cfg(feature = "sqlite-memory")]
    #[test]
    fn sqlite_clipboard_and_voice_history_affect_completion_and_ranking() {
        let context = AppContext::new("com.apple.TextEdit");
        let mut memory = SqliteMemory::open_in_memory(AppPolicy::default()).unwrap();
        memory
            .learn(MemorySource::Voice, "中国市场", &context)
            .unwrap();
        memory
            .learn(MemorySource::Voice, "中国市场", &context)
            .unwrap();
        memory
            .learn(MemorySource::Clipboard, "中国市场报告", &context)
            .unwrap();

        let completions = memory.completion_candidates("中国", 10).unwrap();
        assert!(completions
            .iter()
            .any(|candidate| candidate.text == "中国市场"));
        assert!(completions
            .iter()
            .any(|candidate| candidate.text == "中国市场报告"));

        let ranked = memory
            .rank_candidates(vec![
                Candidate::new("engine-0", "中果"),
                Candidate::new("engine-1", "中国市场"),
                Candidate::new("engine-2", "中国市场报告"),
            ])
            .unwrap();
        assert_eq!(ranked[0].text, "中国市场");
        assert_eq!(ranked[0].source, CandidateSource::Voice);
        assert!(ranked
            .iter()
            .any(|candidate| candidate.source == CandidateSource::Clipboard));
    }

    #[cfg(feature = "sqlite-memory")]
    #[test]
    fn sqlite_memory_imports_handy_history_read_only() {
        let tempdir = tempfile::tempdir().unwrap();
        let history_db = tempdir.path().join("history.db");
        create_history_db(&history_db);

        let context = AppContext::new("com.pais.handy");
        let mut memory = SqliteMemory::open_in_memory(AppPolicy::default()).unwrap();
        let imported = memory
            .import_handy_history(&history_db, &context, 10)
            .unwrap();
        let completions = memory.completion_candidates("中国", 10).unwrap();

        assert_eq!(imported, 2);
        assert!(completions
            .iter()
            .any(|candidate| candidate.text == "中国市场"));
        assert!(completions
            .iter()
            .any(|candidate| candidate.text == "中国市场复盘"));
        assert_eq!(history_row_count(&history_db), 3);
    }

    #[cfg(feature = "sqlite-memory")]
    #[test]
    fn sqlite_memory_imports_clipboard_text_and_skips_sensitive_source_apps() {
        let tempdir = tempfile::tempdir().unwrap();
        let clipboard_db = tempdir.path().join("clipboard.db");
        create_clipboard_db(&clipboard_db);

        let context = AppContext::new("com.pais.handy");
        let mut memory = SqliteMemory::open_in_memory(AppPolicy::default()).unwrap();
        let imported = memory
            .import_handy_clipboard(&clipboard_db, &context, 10)
            .unwrap();
        let completions = memory.completion_candidates("中国", 10).unwrap();

        assert_eq!(imported, 1);
        assert_eq!(memory.term_count().unwrap(), 1);
        assert_eq!(memory.event_count(PrivacyDecision::Excluded).unwrap(), 1);
        assert!(completions
            .iter()
            .any(|candidate| candidate.text == "中国市场报告"));
        let clipboard_candidates = memory.clipboard_candidates(10).unwrap();
        assert_eq!(clipboard_candidates.len(), 1);
        assert_eq!(clipboard_candidates[0].text, "中国市场报告");
        assert_eq!(clipboard_candidates[0].source, CandidateSource::Clipboard);
        assert!(memory
            .completion_candidates("secret", 10)
            .unwrap()
            .is_empty());
        assert_eq!(clipboard_row_count(&clipboard_db), 3);
    }

    #[cfg(feature = "sqlite-memory")]
    fn create_history_db(path: &std::path::Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
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
            rusqlite::params!["a.wav", 1, false, "a", "中国市场", Option::<String>::None],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcription_history (
                file_name, timestamp, saved, title, transcription_text, post_processed_text
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["b.wav", 2, false, "b", "原始文本", Some("中国市场复盘")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transcription_history (
                file_name, timestamp, saved, title, transcription_text, post_processed_text
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params!["c.wav", 3, false, "c", "   ", Option::<String>::None],
        )
        .unwrap();
    }

    #[cfg(feature = "sqlite-memory")]
    fn create_clipboard_db(path: &std::path::Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
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
            rusqlite::params![
                "text",
                "中国市场报告",
                "hash-normal",
                Some("中国市场报告"),
                Some("com.apple.TextEdit"),
                1,
                18
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clipboard_history (
                content_type, content_preview, content_hash, full_text, source_app, created_at, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "text",
                "secret phrase",
                "hash-secret",
                Some("secret phrase"),
                Some("com.1password.1password"),
                2,
                13
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO clipboard_history (
                content_type, content_preview, content_hash, full_text, image_path, created_at, size_bytes
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![
                "image",
                "Image 10x10",
                "hash-image",
                Option::<String>::None,
                Some("clipboard_images/example.png"),
                3,
                100
            ],
        )
        .unwrap();
    }

    #[cfg(feature = "sqlite-memory")]
    fn history_row_count(path: &std::path::Path) -> i64 {
        let conn =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        conn.query_row("SELECT COUNT(*) FROM transcription_history", [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[cfg(feature = "sqlite-memory")]
    fn clipboard_row_count(path: &std::path::Path) -> i64 {
        let conn =
            rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        conn.query_row("SELECT COUNT(*) FROM clipboard_history", [], |row| {
            row.get(0)
        })
        .unwrap()
    }
}
