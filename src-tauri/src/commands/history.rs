use crate::actions::process_transcription_output;
use crate::managers::{
    history::{HistoryManager, PaginatedHistory},
    transcription::TranscriptionManager,
};
use std::sync::Arc;
use tauri::{AppHandle, State};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAttachmentPurpose {
    Active,
    Export,
    Update,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(deny_unknown_fields)]
pub struct HistoryAttachmentLease {
    pub lease_id: String,
    pub instance_id: String,
    pub attachment_id: Option<String>,
    pub purpose: HistoryAttachmentPurpose,
}
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct HistoryAttachmentAccess {
    pub lease: HistoryAttachmentLease,
    pub path: String,
    pub revision: u64,
}
impl From<inputia_handy_runtime::attachment_store::AttachmentLease> for HistoryAttachmentLease {
    fn from(value: inputia_handy_runtime::attachment_store::AttachmentLease) -> Self {
        use inputia_handy_runtime::attachment_store::PinPurpose;
        Self {
            lease_id: value.lease_id,
            instance_id: value.instance_id,
            attachment_id: value.attachment_id,
            purpose: match value.purpose {
                PinPurpose::Active => HistoryAttachmentPurpose::Active,
                PinPurpose::Export => HistoryAttachmentPurpose::Export,
                PinPurpose::Update => HistoryAttachmentPurpose::Update,
            },
        }
    }
}
impl From<HistoryAttachmentLease> for inputia_handy_runtime::attachment_store::AttachmentLease {
    fn from(value: HistoryAttachmentLease) -> Self {
        use inputia_handy_runtime::attachment_store::PinPurpose;
        Self {
            lease_id: value.lease_id,
            instance_id: value.instance_id,
            attachment_id: value.attachment_id,
            purpose: match value.purpose {
                HistoryAttachmentPurpose::Active => PinPurpose::Active,
                HistoryAttachmentPurpose::Export => PinPurpose::Export,
                HistoryAttachmentPurpose::Update => PinPurpose::Update,
            },
        }
    }
}
#[tauri::command]
#[specta::specta]
pub async fn acquire_history_attachment(
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
    expected_revision: Option<u64>,
    operation_id: String,
) -> Result<HistoryAttachmentAccess, String> {
    let service = history_manager
        .attachment_service()
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || {
        let (lease, path, revision) = service.acquire_source_attachment(
            inputia_handy_runtime::source::SourceTable::History,
            id.to_string(),
            expected_revision,
            inputia_handy_runtime::attachment_store::PinPurpose::Active,
            operation_id,
        )?;
        Ok(HistoryAttachmentAccess {
            lease: lease.into(),
            path: path.to_string_lossy().into_owned(),
            revision,
        })
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
#[specta::specta]
pub async fn release_history_attachment(
    history_manager: State<'_, Arc<HistoryManager>>,
    lease: HistoryAttachmentLease,
) -> Result<(), String> {
    // GUI 的资源释放入口不能释放更新/导出事务持有的 pin。
    if !matches!(lease.purpose, HistoryAttachmentPurpose::Active) {
        return Err("Only active playback leases can be released here".into());
    }
    let service = history_manager
        .attachment_service()
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || service.release_attachment(lease.into()))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
#[specta::specta]
pub async fn release_history_attachment_operation(
    history_manager: State<'_, Arc<HistoryManager>>,
    operation_id: String,
) -> Result<(), String> {
    let service = history_manager
        .attachment_service()
        .map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || service.cancel_attachment_acquire(operation_id))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn get_history_entries(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    cursor: Option<i64>,
    limit: Option<usize>,
) -> Result<PaginatedHistory, String> {
    history_manager
        .get_history_entries(cursor, limit)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn toggle_history_entry_saved(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .toggle_saved_status(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn get_audio_file_path(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    file_name: String,
) -> Result<String, String> {
    let id = history_manager
        .entry_id_for_recording(&file_name)
        .map_err(|e| e.to_string())?
        .ok_or("This history entry has no recording")?;
    let service = history_manager
        .attachment_service()
        .map_err(|e| e.to_string())?;
    // 旧客户端无法 release，保守保留到本进程结束；新客户端使用显式 typed lease。
    let operation = format!(
        "legacy-audio-{}",
        inputia_handy_runtime::attachment_store::new_operation_id().map_err(|e| e.to_string())?
    );
    tauri::async_runtime::spawn_blocking(move || {
        service
            .acquire_source_attachment(
                inputia_handy_runtime::source::SourceTable::History,
                id.to_string(),
                None,
                inputia_handy_runtime::attachment_store::PinPurpose::Active,
                operation,
            )
            .map(|(_, path, _)| path.to_string_lossy().into_owned())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
#[specta::specta]
pub async fn delete_history_entry(
    _app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    id: i64,
) -> Result<(), String> {
    history_manager
        .delete_entry(id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
#[specta::specta]
pub async fn retry_history_entry_transcription(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    transcription_manager: State<'_, Arc<TranscriptionManager>>,
    id: i64,
) -> Result<(), String> {
    retry_history_entry_checked(app, &history_manager, &transcription_manager, id, None).await
}

pub(crate) async fn retry_history_entry_checked(
    app: AppHandle,
    history_manager: &HistoryManager,
    transcription_manager: &Arc<TranscriptionManager>,
    id: i64,
    expected_revision: Option<u64>,
) -> Result<(), String> {
    let entry = history_manager
        .get_entry_by_id(id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("History entry {} not found", id))?;

    let service = history_manager
        .attachment_service()
        .map_err(|e| e.to_string())?;
    let operation = format!(
        "retry-audio-{}",
        inputia_handy_runtime::attachment_store::new_operation_id().map_err(|e| e.to_string())?
    );
    let (samples, recording_revision) = tauri::async_runtime::spawn_blocking(move || {
        let result = (|| {
            let (lease, _, revision) = service.acquire_source_attachment(
                inputia_handy_runtime::source::SourceTable::History,
                id.to_string(),
                expected_revision,
                inputia_handy_runtime::attachment_store::PinPurpose::Active,
                operation.clone(),
            )?;
            let bytes = service.read_attachment(lease.clone());
            let released = service.release_attachment(lease);
            let bytes = bytes?;
            released?;
            let samples = decode_retry_recording(&bytes)?;
            Ok::<_, String>((samples, revision))
        })();
        if result.is_err() {
            let _ = service.cancel_attachment_acquire(operation);
        }
        result
    })
    .await
    .map_err(|e| e.to_string())??;

    if samples.is_empty() {
        return Err("Recording has no audio samples".to_string());
    }

    transcription_manager.initiate_model_load();

    let tm = Arc::clone(transcription_manager);
    let transcription = tauri::async_runtime::spawn_blocking(move || tm.transcribe(samples))
        .await
        .map_err(|e| format!("Transcription task panicked: {}", e))?
        .map_err(|e| e.to_string())?;

    if transcription.is_empty() {
        return Err("Recording contains no speech".to_string());
    }

    let processed =
        process_transcription_output(&app, &transcription, entry.post_process_requested).await;
    history_manager
        .update_transcription_checked(
            id,
            transcription,
            processed.post_processed_text,
            processed.post_process_prompt,
            Some(recording_revision),
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// 读取由受管租约验证的同一文件描述符字节，避免校验路径后重新打开。
fn decode_retry_recording(bytes: &[u8]) -> Result<Vec<f32>, String> {
    let reader = hound::WavReader::new(std::io::Cursor::new(bytes))
        .map_err(|_| "Recording format is invalid".to_owned())?;
    if reader.spec().channels != 1
        || reader.spec().sample_rate != 16000
        || reader.spec().bits_per_sample != 16
    {
        return Err("Recording format is unsupported".into());
    }
    reader
        .into_samples::<i16>()
        .map(|sample| {
            sample
                .map(|value| value as f32 / i16::MAX as f32)
                .map_err(|_| "Recording data is invalid".into())
        })
        .collect()
}
#[cfg(test)]
mod tests {
    #[test]
    fn retries_decode_validated_recording_bytes() {
        let mut output = std::io::Cursor::new(Vec::new());
        {
            let mut writer = hound::WavWriter::new(
                &mut output,
                hound::WavSpec {
                    channels: 1,
                    sample_rate: 16000,
                    bits_per_sample: 16,
                    sample_format: hound::SampleFormat::Int,
                },
            )
            .unwrap();
            writer.write_sample(123i16).unwrap();
            writer.finalize().unwrap();
        }
        assert_eq!(
            super::decode_retry_recording(output.get_ref())
                .unwrap()
                .len(),
            1
        );
        assert!(super::decode_retry_recording(b"not a WAV").is_err());
    }
}

#[tauri::command]
#[specta::specta]
pub async fn update_history_limit(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    limit: usize,
) -> Result<(), String> {
    let mut settings = crate::settings::get_settings(&app);
    settings.history_limit = limit;
    crate::settings::write_settings(&app, settings)?;

    history_manager
        .cleanup_old_entries()
        .map_err(|_| "settings_saved_history_cleanup_failed".to_owned())?;

    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn update_recording_retention_period(
    app: AppHandle,
    history_manager: State<'_, Arc<HistoryManager>>,
    period: String,
) -> Result<(), String> {
    use crate::settings::RecordingRetentionPeriod;

    let retention_period = match period.as_str() {
        "never" => RecordingRetentionPeriod::Never,
        "preserve_limit" => RecordingRetentionPeriod::PreserveLimit,
        "days3" => RecordingRetentionPeriod::Days3,
        "weeks2" => RecordingRetentionPeriod::Weeks2,
        "months3" => RecordingRetentionPeriod::Months3,
        _ => return Err(format!("Invalid retention period: {}", period)),
    };

    let mut settings = crate::settings::get_settings(&app);
    settings.recording_retention_period = retention_period;
    crate::settings::write_settings(&app, settings)?;

    history_manager
        .cleanup_old_entries()
        .map_err(|_| "settings_saved_history_cleanup_failed".to_owned())?;

    Ok(())
}
