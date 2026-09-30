//! 外部编辑必须先展示，再用原始字节摘要确认导入；丢失/坏元数据不隐式重置。
use super::*;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExternalSnapshot {
    pub store_id: String,
    pub revision: String,
    pub observed_file_digest: String,
    pub values: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRequest {
    pub operation_id: String,
    pub expected_store_id: String,
    pub expected_revision: String,
    pub observed_file_digest: String,
}
impl Store {
    pub fn inspect_external(&self) -> Result<ExternalSnapshot> {
        let document = self.load_internal(true)?;
        if digest(&Value::Object(document.values.clone()))? == document.header.values_digest {
            return Err(Error::InvalidRequest);
        }
        Ok(ExternalSnapshot {
            store_id: document.header.store_id,
            revision: document.header.revision,
            observed_file_digest: document.source_digest.ok_or(Error::RepairRequired)?,
            values: Value::Object(document.values),
        })
    }
    pub fn import_external(&self, request: &ImportRequest) -> Result<ApplyResult> {
        let expected = revision(&request.expected_revision).map_err(|_| Error::InvalidRequest)?;
        if operation_base(&request.operation_id, &request.expected_store_id)? != expected
            || !is_digest(&request.observed_file_digest)
        {
            return Err(Error::InvalidRequest);
        }
        let request_digest =
            digest(&serde_json::json!({"action":"import_external","request":request}))?;
        let mut document = self.load_internal(true)?;
        if request.expected_store_id != document.header.store_id {
            return Err(Error::RepairRequired);
        }
        if let Some(receipt) = document
            .header
            .receipts
            .iter()
            .find(|r| r.operation_id == request.operation_id)
        {
            if receipt.request_digest != request_digest {
                return Err(Error::OperationMismatch);
            }
            // 导入曾完成但回复丢失：正文又被外部编辑时不能把旧 receipt 冒充当前文件已保存。
            if digest(&Value::Object(document.values.clone()))? != document.header.values_digest {
                return Err(Error::ExternalEdit);
            }
            self.files.confirm_durable(&mut |_| Ok(()))?;
            return Ok(ApplyResult::Saved {
                commit_revision: receipt.revision.clone(),
                replayed: true,
                current: document.snapshot(),
            });
        }
        if expected != revision(&document.header.revision)?
            || document.source_digest.as_ref() != Some(&request.observed_file_digest)
        {
            return Err(Error::ExternalChanged);
        }
        let next_digest = digest(&Value::Object(document.values.clone()))?;
        if next_digest == document.header.values_digest {
            return Err(Error::InvalidRequest);
        }
        document.header.revision = expected
            .checked_add(1)
            .ok_or(Error::RevisionExhausted)?
            .to_string();
        document.header.values_digest = next_digest;
        document.header.receipts.push(Receipt {
            operation_id: request.operation_id.clone(),
            request_digest,
            revision: document.header.revision.clone(),
            values_digest: document.header.values_digest.clone(),
        });
        if document.header.receipts.len() > RECEIPT_LIMIT {
            document.header.receipts.remove(0);
        }
        maintenance::ensure_normal_start(&self.home, self.uid).map_err(|_| Error::Maintenance)?;
        self.files
            .replace("settings.json", &document.bytes()?, &mut |_| Ok(()))?;
        Ok(ApplyResult::Saved {
            commit_revision: document.header.revision.clone(),
            replayed: false,
            current: document.snapshot(),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preview_is_read_only_and_changed_preview_cannot_be_imported() {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path().canonicalize().unwrap();
        let path = home.join("settings.json");
        let store = Store::open(&path, &home, unsafe { libc::geteuid() }).unwrap();
        let initial = store.read().unwrap();
        let mut raw = strict_json(&std::fs::read(&path).unwrap()).unwrap();
        raw["memory_enabled"] = Value::Bool(false);
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        let before = std::fs::read(&path).unwrap();
        let preview = store.inspect_external().unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let mut request = ImportRequest {
            operation_id: initial.operation_id(),
            expected_store_id: preview.store_id,
            expected_revision: preview.revision,
            observed_file_digest: preview.observed_file_digest,
        };
        raw["candidate_page_size"] = 5.into();
        std::fs::write(&path, serde_json::to_vec(&raw).unwrap()).unwrap();
        assert_eq!(
            store.import_external(&request).unwrap_err(),
            Error::ExternalChanged
        );
        request.observed_file_digest = store.inspect_external().unwrap().observed_file_digest;
        assert!(
            matches!(store.import_external(&request).unwrap(), ApplyResult::Saved {replayed:false,commit_revision,..} if commit_revision=="1")
        );
        assert_eq!(store.read().unwrap().values["memory_enabled"], false);
        assert!(matches!(
            store.import_external(&request).unwrap(),
            ApplyResult::Saved { replayed: true, .. }
        ));
        request.observed_file_digest = "0".repeat(64);
        assert_eq!(
            store.import_external(&request).unwrap_err(),
            Error::OperationMismatch
        );
    }
}
