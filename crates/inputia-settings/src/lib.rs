use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InputiaSettings {
    pub schema_id: String,
    pub candidate_page_size: usize,
    pub candidate_font_size: usize,
    pub menu_icon_variant: String,
    pub shift_toggle_enabled: bool,
    pub input_mode_toggle_shortcut: InputModeToggleShortcut,
    pub chinese_script: ChineseScript,
    pub script_toggle_shortcut: ScriptToggleShortcut,
    pub punctuation_preference: PunctuationPreference,
    pub character_width_preference: CharacterWidthPreference,
    #[serde(default = "default_true")]
    pub spelling_correction_enabled: bool,
    pub memory_enabled: bool,
    pub privacy_learning_enabled: bool,
    pub sensitive_bundle_ids: Vec<String>,
    pub rime_dylib_path: Option<PathBuf>,
    pub rime_shared_data_dir: Option<PathBuf>,
    pub rime_user_data_dir: Option<PathBuf>,
    pub memory_db_path: Option<PathBuf>,
}

impl Default for InputiaSettings {
    fn default() -> Self {
        Self {
            schema_id: "luna_pinyin_simp".to_string(),
            candidate_page_size: 7,
            candidate_font_size: 14,
            menu_icon_variant: default_menu_icon_variant(),
            shift_toggle_enabled: true,
            input_mode_toggle_shortcut: InputModeToggleShortcut::Shift,
            chinese_script: ChineseScript::Simplified,
            script_toggle_shortcut: ScriptToggleShortcut::ControlShiftS,
            punctuation_preference: PunctuationPreference::EnglishInChinese,
            character_width_preference: CharacterWidthPreference::HalfWidth,
            spelling_correction_enabled: true,
            memory_enabled: true,
            privacy_learning_enabled: true,
            sensitive_bundle_ids: default_sensitive_bundle_ids(),
            rime_dylib_path: None,
            rime_shared_data_dir: None,
            rime_user_data_dir: None,
            memory_db_path: None,
        }
    }
}

impl InputiaSettings {
    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            return Self::load(path);
        }

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let settings = Self::default_for_settings_path(path);
        settings.save(path)?;
        Ok(settings)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let content = std::fs::read_to_string(path)?;
        let mut settings: Self = serde_json::from_str(&content)?;
        settings.sanitize_for_settings_path(path);
        Ok(settings)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = serde_json::to_string_pretty(self)?;
        std::fs::write(path, format!("{content}\n"))?;
        Ok(())
    }

    pub fn default_for_settings_path(path: &Path) -> Self {
        let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
        let mut settings = Self::default();
        settings.rime_user_data_dir = Some(base_dir.join("rime"));
        settings.memory_db_path = Some(base_dir.join("inputia_memory.db"));
        settings
    }

    pub fn sanitize(&mut self) {
        if self.schema_id.trim().is_empty() {
            self.schema_id = Self::default().schema_id;
        }
        if !self.shift_toggle_enabled
            && self.input_mode_toggle_shortcut == InputModeToggleShortcut::Shift
        {
            self.input_mode_toggle_shortcut = InputModeToggleShortcut::None;
        }
        self.shift_toggle_enabled =
            self.input_mode_toggle_shortcut == InputModeToggleShortcut::Shift;
        self.candidate_page_size = self.candidate_page_size.clamp(1, 9);
        self.candidate_font_size = self.candidate_font_size.clamp(12, 22);
        if !is_known_menu_icon_variant(&self.menu_icon_variant) {
            self.menu_icon_variant = default_menu_icon_variant();
        }
        if self.sensitive_bundle_ids.is_empty() {
            self.sensitive_bundle_ids = default_sensitive_bundle_ids();
        }
    }

    pub fn sanitize_for_settings_path(&mut self, path: &Path) {
        self.sanitize();
        let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
        if self.rime_user_data_dir.is_none() {
            self.rime_user_data_dir = Some(base_dir.join("rime"));
        }
        if self.memory_db_path.is_none() {
            self.memory_db_path = Some(base_dir.join("inputia_memory.db"));
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChineseScript {
    #[default]
    Simplified,
    Traditional,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PunctuationPreference {
    FollowInputMode,
    EnglishInChinese,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CharacterWidthPreference {
    HalfWidth,
    FullWidth,
}

impl Default for CharacterWidthPreference {
    fn default() -> Self {
        Self::HalfWidth
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InputModeToggleShortcut {
    #[default]
    Shift,
    ControlSpace,
    None,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScriptToggleShortcut {
    #[default]
    ControlShiftS,
    None,
}

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::Json(error) => write!(formatter, "JSON error: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for Error {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn default_sensitive_bundle_ids() -> Vec<String> {
    vec![
        "com.1password.1password".to_string(),
        "com.agilebits.onepassword7".to_string(),
        "com.apple.Safari.PrivateBrowsing".to_string(),
        "com.apple.SecurityAgent".to_string(),
        "com.bitwarden.desktop".to_string(),
        "com.lastpass.LastPass".to_string(),
        "com.protonmail.protonmail".to_string(),
    ]
}

fn default_true() -> bool {
    true
}

fn default_menu_icon_variant() -> String {
    "pearl_16".to_string()
}

fn is_known_menu_icon_variant(value: &str) -> bool {
    matches!(value, "pearl_12" | "pearl_14" | "pearl_16" | "pearl_18")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_or_create_writes_local_default_paths() {
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");

        let settings = InputiaSettings::load_or_create(&settings_path).unwrap();

        assert!(settings_path.exists());
        assert_eq!(settings.schema_id, "luna_pinyin_simp");
        assert_eq!(settings.candidate_page_size, 7);
        assert_eq!(settings.candidate_font_size, 14);
        assert_eq!(settings.menu_icon_variant, "pearl_16");
        assert!(settings.shift_toggle_enabled);
        assert_eq!(
            settings.input_mode_toggle_shortcut,
            InputModeToggleShortcut::Shift
        );
        assert!(settings.spelling_correction_enabled);
        assert_eq!(settings.rime_user_data_dir, Some(temp.path().join("rime")));
        assert_eq!(
            settings.memory_db_path,
            Some(temp.path().join("inputia_memory.db"))
        );
        assert!(settings
            .sensitive_bundle_ids
            .iter()
            .any(|bundle_id| bundle_id == "com.apple.SecurityAgent"));
    }

    #[test]
    fn load_sanitizes_candidate_page_size_and_empty_sensitive_list() {
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        std::fs::write(
            &settings_path,
            r#"{
              "schema_id": "",
              "candidate_page_size": 99,
              "candidate_font_size": 99,
              "menu_icon_variant": "not-a-real-icon",
              "sensitive_bundle_ids": []
            }"#,
        )
        .unwrap();

        let settings = InputiaSettings::load(&settings_path).unwrap();

        assert_eq!(settings.schema_id, "luna_pinyin_simp");
        assert_eq!(settings.candidate_page_size, 9);
        assert_eq!(settings.candidate_font_size, 22);
        assert_eq!(settings.menu_icon_variant, "pearl_16");
        assert_eq!(
            settings.input_mode_toggle_shortcut,
            InputModeToggleShortcut::Shift
        );
        assert!(settings.spelling_correction_enabled);
        assert!(settings
            .sensitive_bundle_ids
            .iter()
            .any(|bundle_id| bundle_id == "com.1password.1password"));
    }

    #[test]
    fn explicit_overrides_round_trip() {
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        let settings = InputiaSettings {
            schema_id: "double_pinyin_flypy".to_string(),
            candidate_page_size: 3,
            candidate_font_size: 18,
            menu_icon_variant: "pearl_14".to_string(),
            shift_toggle_enabled: false,
            input_mode_toggle_shortcut: InputModeToggleShortcut::ControlSpace,
            punctuation_preference: PunctuationPreference::FollowInputMode,
            character_width_preference: CharacterWidthPreference::FullWidth,
            spelling_correction_enabled: false,
            memory_enabled: false,
            privacy_learning_enabled: false,
            ..InputiaSettings::default()
        };

        settings.save(&settings_path).unwrap();
        let loaded = InputiaSettings::load(&settings_path).unwrap();

        assert_eq!(loaded.schema_id, "double_pinyin_flypy");
        assert_eq!(loaded.candidate_page_size, 3);
        assert_eq!(loaded.candidate_font_size, 18);
        assert_eq!(loaded.menu_icon_variant, "pearl_14");
        assert!(!loaded.shift_toggle_enabled);
        assert_eq!(
            loaded.input_mode_toggle_shortcut,
            InputModeToggleShortcut::ControlSpace
        );
        assert_eq!(
            loaded.punctuation_preference,
            PunctuationPreference::FollowInputMode
        );
        assert_eq!(
            loaded.character_width_preference,
            CharacterWidthPreference::FullWidth
        );
        assert!(!loaded.spelling_correction_enabled);
        assert!(!loaded.memory_enabled);
        assert!(!loaded.privacy_learning_enabled);
    }

    #[test]
    fn legacy_disabled_shift_migrates_to_no_input_mode_shortcut() {
        let temp = tempfile::tempdir().unwrap();
        let settings_path = temp.path().join("settings.json");
        std::fs::write(
            &settings_path,
            r#"{
              "shift_toggle_enabled": false
            }"#,
        )
        .unwrap();

        let settings = InputiaSettings::load(&settings_path).unwrap();

        assert!(!settings.shift_toggle_enabled);
        assert_eq!(
            settings.input_mode_toggle_shortcut,
            InputModeToggleShortcut::None
        );
    }
}
