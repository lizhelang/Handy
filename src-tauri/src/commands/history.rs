use crate::actions::process_transcription_output;
use crate::managers::{
    history::{HistoryManager, PaginatedHistory},
    transcription::TranscriptionManager,
};
use std::sync::Arc;
use tauri::{AppHandle, State};

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
    if file_name.is_empty() {
        return Err("This history entry has no recording".to_owned());
    }
    let path = history_manager.get_audio_file_path(&file_name);
    path.to_str()
        .ok_or_else(|| "Invalid file path".to_string())
        .map(|s| s.to_string())
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

    let samples =
        read_retry_recording(history_manager.recordings_dir(), &entry.file_name, |path| {
            crate::audio_toolkit::read_wav_samples(path)
        })?;

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
            expected_revision,
        )
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// 重转写的实际读取边界：无附件在路径解析、目录读取和模型启动之前返回。
fn read_retry_recording(
    recordings: &std::path::Path,
    file_name: &str,
    read: impl FnOnce(&std::path::Path) -> anyhow::Result<Vec<f32>>,
) -> Result<Vec<f32>, String> {
    if file_name.is_empty() {
        return Err("This history entry has no recording".into());
    }
    let audio_path = recordings
        .join(file_name)
        .canonicalize()
        .map_err(|_| "Recording file is unavailable".to_owned())?;
    let recordings = recordings
        .canonicalize()
        .map_err(|_| "Recordings directory is unavailable".to_owned())?;
    if !audio_path.starts_with(recordings) {
        return Err("Recording is outside managed storage".into());
    }
    read(&audio_path).map_err(|error| format!("Failed to load audio: {error}"))
}

#[cfg(test)]
mod tests {
    use super::read_retry_recording;
    #[test]
    fn empty_attachment_retranscription_never_reads_the_recordings_directory() {
        let temp = tempfile::tempdir().unwrap();
        let mut reads = 0;
        let result = read_retry_recording(temp.path(), "", |_| {
            reads += 1;
            Ok(vec![0.1])
        });
        assert!(result.is_err());
        assert_eq!(reads, 0);
        assert!(temp.path().is_dir());
    }

    #[test]
    fn valid_managed_attachment_is_read_for_retranscription() {
        let temp = tempfile::tempdir().unwrap();
        let wav = temp.path().join("synthetic.wav");
        crate::audio_toolkit::save_wav_file(&wav, &[0.1, -0.2, 0.3]).unwrap();
        let mut reads = 0;
        let samples = read_retry_recording(temp.path(), "synthetic.wav", |path| {
            reads += 1;
            assert_eq!(path, wav.canonicalize().unwrap());
            crate::audio_toolkit::read_wav_samples(path)
        })
        .unwrap();
        assert_eq!(reads, 1);
        assert_eq!(samples.len(), 3);
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
    crate::settings::write_settings(&app, settings);

    history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

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
    crate::settings::write_settings(&app, settings);

    history_manager
        .cleanup_old_entries()
        .map_err(|e| e.to_string())?;

    Ok(())
}
