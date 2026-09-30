//! 控制中心设置的领域适配与字段级修改计划；运行时 writer 的切换另由 settings 模块协调。
use super::*;
use inputia_settings::store::{
    ApplyResult, DocumentSchema, DocumentStore, Error, PatchRequest, Snapshot,
};
use serde_json::{Map, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

pub(super) struct Schema;
impl DocumentSchema for Schema {
    const FILE_NAME: &'static str = SETTINGS_STORE_PATH;
    const MARKER_NAME: &'static str = ".inputia-control-settings-initialized.json";
    const DOMAIN: &'static str = "inputia.control-settings";
    const PATCH_ROOT: &'static [&'static str] = &["settings"];

    fn defaults(_: &Path) -> Result<Map<String, Value>, Error> {
        Ok(Map::from_iter([(
            "settings".into(),
            encode(&get_default_settings())?,
        )]))
    }
    fn validate(
        values: &Map<String, Value>,
        _: &Path,
        migrate: bool,
    ) -> Result<Map<String, Value>, Error> {
        let raw = values
            .get("settings")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let original = raw.as_object().ok_or(Error::InvalidDocument)?;
        // 坏值保留原文件等待修复；不能因一项解析失败清空用户的其他配置或凭据。
        let mut parsed: AppSettings =
            serde_json::from_value(raw.clone()).map_err(|_| Error::InvalidDocument)?;
        if parsed.settings_schema_version > CURRENT_SETTINGS_SCHEMA_VERSION {
            return Err(Error::InvalidDocument);
        }
        for ids in [
            parsed
                .post_process_providers
                .iter()
                .map(|v| v.id.as_str())
                .collect::<Vec<_>>(),
            parsed
                .post_process_prompts
                .iter()
                .map(|v| v.id.as_str())
                .collect::<Vec<_>>(),
        ] {
            let mut seen = std::collections::BTreeSet::new();
            if ids.into_iter().any(|id| {
                id.is_empty()
                    || id.len() > 256
                    || id.chars().any(char::is_control)
                    || !seen.insert(id)
            }) {
                return Err(Error::InvalidDocument);
            }
        }
        if migrate {
            apply_settings_migrations(&mut parsed, &raw);
            merge_missing_bindings(&mut parsed);
            ensure_post_process_defaults(&mut parsed);
        }
        let encoded = encode(&parsed)?;
        let known = encoded.as_object().ok_or(Error::InvalidDocument)?;
        let mut settings = original.clone();
        for (key, value) in known {
            settings.insert(
                key.clone(),
                preserve_field_extensions(key, original.get(key), value),
            );
        }
        let mut result = values.clone();
        result.insert("settings".into(), Value::Object(settings));
        Ok(result)
    }
}

fn encode(settings: &AppSettings) -> Result<Value, Error> {
    // JSON f64 有限仍可能在解码 f32 时溢出；serde 编码 infinity 会变成 null。
    if !settings.audio_feedback_volume.is_finite()
        || !settings.word_correction_threshold.is_finite()
    {
        return Err(Error::InvalidDocument);
    }
    serde_json::to_value(settings).map_err(|_| Error::InvalidDocument)
}
fn extend_object(original: Option<&Value>, known: &Value) -> Value {
    let Some(known) = known.as_object() else {
        return known.clone();
    };
    let mut merged = original
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    merged.extend(known.clone());
    Value::Object(merged)
}
fn preserve_field_extensions(key: &str, original: Option<&Value>, known: &Value) -> Value {
    match key {
        "bindings" => known
            .as_object()
            .map(|bindings| {
                Value::Object(
                    bindings
                        .iter()
                        .map(|(id, value)| {
                            (
                                id.clone(),
                                extend_object(original.and_then(|v| v.get(id)), value),
                            )
                        })
                        .collect(),
                )
            })
            .unwrap_or_else(|| known.clone()),
        "post_process_providers" | "post_process_prompts" => {
            let Some(items) = known.as_array() else {
                return known.clone();
            };
            let prior = original.and_then(Value::as_array);
            Value::Array(
                items
                    .iter()
                    .map(|value| {
                        let id = value.get("id").and_then(Value::as_str);
                        let old = id.and_then(|id| {
                            prior.and_then(|items| {
                                items
                                    .iter()
                                    .find(|v| v.get("id").and_then(Value::as_str) == Some(id))
                            })
                        });
                        extend_object(old, value)
                    })
                    .collect(),
            )
        }
        // 动态 map 的删除必须生效，例如删除 provider 的 API key；不把被删除项当未知扩展恢复。
        _ => known.clone(),
    }
}

/// 原始快照与文件位置不向 wire 反序列化，修改计划只从这次真实读取产生。
pub(super) struct LoadedSettings {
    snapshot: Snapshot,
    settings: AppSettings,
    path: PathBuf,
    home: PathBuf,
    uid: u32,
}
pub(super) struct PlannedChange {
    request: PatchRequest,
    path: PathBuf,
    home: PathBuf,
    uid: u32,
}
pub(super) enum SavedSettings {
    Saved {
        current: LoadedSettings,
        commit_revision: String,
        replayed: bool,
    },
    Conflict {
        current: LoadedSettings,
    },
    OutcomeExpired {
        current: LoadedSettings,
    },
}
impl LoadedSettings {
    pub fn reload(&self) -> Result<Self, Error> {
        let snapshot = DocumentStore::<Schema>::open(&self.path, &self.home, self.uid)?
            .read_at_least(&self.snapshot)?;
        Self::from_snapshot(snapshot, self.path.clone(), self.home.clone(), self.uid)
    }
    pub fn follows(&self, previous: &Self) -> bool {
        self.path == previous.path
            && self.home == previous.home
            && self.uid == previous.uid
            && self.snapshot.store_id == previous.snapshot.store_id
            && self.snapshot.revision.parse::<u64>().is_ok_and(|revision| {
                previous
                    .snapshot
                    .revision
                    .parse::<u64>()
                    .is_ok_and(|prior| {
                        revision > prior
                            || (revision == prior
                                && self.snapshot.values_digest == previous.snapshot.values_digest)
                    })
            })
    }
    pub fn revision(&self) -> &str {
        &self.snapshot.revision
    }
    pub fn settings(&self) -> &AppSettings {
        &self.settings
    }
    fn from_snapshot(
        snapshot: Snapshot,
        path: PathBuf,
        home: PathBuf,
        uid: u32,
    ) -> Result<Self, Error> {
        if snapshot.domain() != Schema::DOMAIN {
            return Err(Error::InvalidDocument);
        }
        let settings = serde_json::from_value(
            snapshot
                .values
                .get("settings")
                .cloned()
                .ok_or(Error::InvalidDocument)?,
        )
        .map_err(|_| Error::InvalidDocument)?;
        Ok(Self {
            snapshot,
            settings,
            path,
            home,
            uid,
        })
    }
    pub fn read(path: &Path, home: &Path, uid: u32) -> Result<Self, Error> {
        let snapshot = DocumentStore::<Schema>::open(path, home, uid)?.read()?;
        Self::from_snapshot(snapshot, path.into(), home.into(), uid)
    }
    pub fn plan(&self, edited: &AppSettings) -> Result<Option<PlannedChange>, Error> {
        let before = encode(&self.settings)?;
        let after = encode(edited)?;
        let mut patch = BTreeMap::new();
        for (key, value) in after.as_object().ok_or(Error::InvalidDocument)? {
            if before.get(key) != Some(value) {
                patch.insert(
                    key.clone(),
                    preserve_field_extensions(
                        key,
                        self.snapshot
                            .values
                            .get("settings")
                            .and_then(|v| v.get(key)),
                        value,
                    ),
                );
            }
        }
        if patch.is_empty() {
            return Ok(None);
        }
        Ok(Some(PlannedChange {
            request: PatchRequest {
                operation_id: self.snapshot.operation_id(),
                expected_store_id: self.snapshot.store_id.clone(),
                expected_revision: self.snapshot.revision.clone(),
                patch,
            },
            path: self.path.clone(),
            home: self.home.clone(),
            uid: self.uid,
        }))
    }
}
impl PlannedChange {
    pub fn operation_id(&self) -> &str {
        &self.request.operation_id
    }
    pub fn changed_fields(&self) -> Vec<String> {
        self.request.patch.keys().cloned().collect()
    }
    pub fn apply_at_least(&self, floor: &LoadedSettings) -> Result<SavedSettings, Error> {
        if self.path != floor.path || self.home != floor.home || self.uid != floor.uid {
            return Err(Error::RepairRequired);
        }
        let result = DocumentStore::<Schema>::open(&self.path, &self.home, self.uid)?
            .apply_at_least(&self.request, &floor.snapshot)?;
        self.result(result)
    }
    /// 同一实例可按原 ID 重试；错误不清除原请求或捏造成功。
    pub fn apply(&self) -> Result<SavedSettings, Error> {
        let result = DocumentStore::<Schema>::open(&self.path, &self.home, self.uid)?
            .apply(&self.request)?;
        self.result(result)
    }
    fn result(&self, result: ApplyResult) -> Result<SavedSettings, Error> {
        let loaded = |snapshot| {
            LoadedSettings::from_snapshot(snapshot, self.path.clone(), self.home.clone(), self.uid)
        };
        Ok(match result {
            ApplyResult::Saved {
                current,
                commit_revision,
                replayed,
            } => SavedSettings::Saved {
                current: loaded(current)?,
                commit_revision,
                replayed,
            },
            ApplyResult::Conflict { current } => SavedSettings::Conflict {
                current: loaded(current)?,
            },
            ApplyResult::OutcomeExpired { current } => SavedSettings::OutcomeExpired {
                current: loaded(current)?,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf, u32) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().canonicalize().unwrap();
        let path = home.join(SETTINGS_STORE_PATH);
        (temp, home, path, unsafe { libc::geteuid() })
    }
    #[test]
    fn app_delta_preserves_unknown_fields_and_conflicting_writers_cannot_clobber() {
        let (_temp, home, path, uid) = fixture();
        let mut raw = json!({"settings":get_default_settings(),"root_extension":{"future":true}});
        raw["settings"]["nested_extension"] = json!([1, 2]);
        let provider = raw["settings"]["post_process_providers"]
            .as_array_mut()
            .unwrap()
            .first_mut()
            .unwrap();
        provider["future_provider_flag"] = json!(true);
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let first = LoadedSettings::read(&path, &home, uid).unwrap();
        let second = LoadedSettings::read(&path, &home, uid).unwrap();
        let mut theme = first.settings.clone();
        theme.theme = Theme::Dark;
        let plan = first.plan(&theme).unwrap().unwrap();
        assert_eq!(plan.request.patch.len(), 1);
        assert!(plan.request.patch.contains_key("theme"));
        let SavedSettings::Saved {
            current,
            replayed: false,
            ..
        } = plan.apply().unwrap()
        else {
            panic!("must save")
        };
        assert_eq!(current.snapshot.values["root_extension"]["future"], true);
        assert_eq!(
            current.snapshot.values["settings"]["nested_extension"],
            json!([1, 2])
        );
        let mut stale = second.settings.clone();
        stale.audio_feedback = !stale.audio_feedback;
        assert!(matches!(
            second.plan(&stale).unwrap().unwrap().apply().unwrap(),
            SavedSettings::Conflict { .. }
        ));
        assert!(matches!(
            plan.apply().unwrap(),
            SavedSettings::Saved { replayed: true, .. }
        ));
        let mut provider_edit = current.settings.clone();
        provider_edit.post_process_providers[0].label = "renamed".into();
        let SavedSettings::Saved { current, .. } = current
            .plan(&provider_edit)
            .unwrap()
            .unwrap()
            .apply()
            .unwrap()
        else {
            panic!("must save")
        };
        assert_eq!(
            current.snapshot.values["settings"]["post_process_providers"][0]
                ["future_provider_flag"],
            true
        );
    }
    #[test]
    fn malformed_or_future_settings_do_not_get_replaced_by_defaults() {
        for settings in [
            json!({"post_process_api_keys":{"fixture":42}}),
            json!({"settings_schema_version":999}),
        ] {
            let (_temp, home, path, uid) = fixture();
            let bytes =
                serde_json::to_vec(&json!({"settings":settings,"other":"preserve"})).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            assert!(LoadedSettings::read(&path, &home, uid).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }
    #[test]
    fn removing_dynamic_map_entries_does_not_restore_them_as_extensions() {
        let (_temp, home, path, uid) = fixture();
        let mut settings = get_default_settings();
        settings
            .post_process_api_keys
            .insert("kept".into(), "synthetic".into());
        settings
            .post_process_api_keys
            .insert("removed".into(), "synthetic".into());
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({"settings":settings})).unwrap(),
        )
        .unwrap();
        let loaded = LoadedSettings::read(&path, &home, uid).unwrap();
        let mut edited = loaded.settings().clone();
        edited.post_process_api_keys.remove("removed");
        let SavedSettings::Saved { current, .. } =
            loaded.plan(&edited).unwrap().unwrap().apply().unwrap()
        else {
            panic!("must save")
        };
        assert!(!current
            .settings()
            .post_process_api_keys
            .contains_key("removed"));
        assert_eq!(
            current
                .settings()
                .post_process_api_keys
                .get("kept")
                .map(String::as_str),
            Some("synthetic")
        );
        assert!(current.snapshot.values["settings"]["post_process_api_keys"]
            .get("removed")
            .is_none());
    }

    #[test]
    fn external_float_overflow_and_in_memory_non_finite_edits_are_rejected() {
        use inputia_settings::store::{strict_json, ImportRequest};
        use sha2::{Digest, Sha256};
        let (_temp, home, path, uid) = fixture();
        let loaded = LoadedSettings::read(&path, &home, uid).unwrap();
        for value in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            let mut edited = loaded.settings().clone();
            edited.audio_feedback_volume = value;
            assert!(matches!(loaded.plan(&edited), Err(Error::InvalidDocument)));
        }
        let mut edited = loaded.settings().clone();
        edited.word_correction_threshold = f64::NAN;
        assert!(matches!(loaded.plan(&edited), Err(Error::InvalidDocument)));
        let mut raw = strict_json(&std::fs::read(&path).unwrap()).unwrap();
        raw["settings"]["audio_feedback_volume"] = json!(1e300);
        let bytes = serde_json::to_vec(&raw).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let store = DocumentStore::<Schema>::open(&path, &home, uid).unwrap();
        assert!(matches!(
            store.inspect_external(),
            Err(Error::InvalidDocument)
        ));
        assert!(matches!(
            store.import_external(&ImportRequest {
                operation_id: loaded.snapshot.operation_id(),
                expected_store_id: loaded.snapshot.store_id.clone(),
                expected_revision: loaded.snapshot.revision.clone(),
                observed_file_digest: format!("{:x}", Sha256::digest(&bytes)),
            }),
            Err(Error::InvalidDocument)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
