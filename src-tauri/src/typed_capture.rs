//! 输入法正文的唯一写入入口，使用已认证连接与实时非敏感字段证明。

use inputia_handy_runtime::{
    typed_history,
    voice_protocol::{
        TargetBridgePurpose, TypedCaptureCommand, TypedCaptureReply, TypedCaptureRequest,
    },
};

/// 调用方已验证 socket 对端、请求世代和策略屏障。永不在日志中输出正文。
pub fn respond(app: &tauri::AppHandle, request: &TypedCaptureRequest) -> TypedCaptureReply {
    let mut reply = TypedCaptureReply {
        status: "typed_capture".into(),
        request_id: request.request_id.clone(),
        server_instance: request.server_instance.clone(),
        enabled: false,
        epoch: 0,
        saved: false,
        code: None,
    };
    let result = (|| -> Result<(), String> {
        let root = crate::portable::app_data_dir(app).map_err(|e| e.to_string())?;
        let policy = typed_history::policy(&root)?;
        reply.enabled = policy.enabled;
        reply.epoch = policy.epoch;
        let TypedCaptureCommand::Commit {
            capture_epoch,
            event_id,
            segment_id,
            text,
            draft,
        } = &request.typed_capture
        else {
            return Ok(());
        };
        if !policy.enabled || policy.epoch != *capture_epoch {
            return Err("typed_capture_disabled_or_changed".into());
        }
        if crate::secure_input::is_enabled_now() {
            return Err("typed_capture_secure_input".into());
        }
        // 必须携带产生文本前签发的字段令牌，不能事后捕获另一个字段来证明旧正文。
        if draft.field_id.as_deref() != Some(draft.target_id.as_str()) {
            return Err("typed_capture_origin_unverified".into());
        }
        let permission_epoch = crate::input_permission::capture_epoch()?;
        crate::ime_target_broker::check(
            app,
            &request.client_instance,
            &request.server_instance,
            draft,
            TargetBridgePurpose::TypedCapture,
        )?;
        crate::input_permission::check_epoch(permission_epoch)?;
        if crate::secure_input::is_enabled_now() {
            return Err("typed_capture_secure_input".into());
        }
        // 原字段与进程实例同时参与片段身份，不能因同应用的另一个字段而混段。
        typed_history::record(
            &root,
            &format!("{}:{event_id}", request.client_instance),
            &format!(
                "{}:{}:{segment_id}",
                request.client_instance, draft.target_id
            ),
            text,
            draft
                .source_app
                .as_deref()
                .ok_or("typed_capture_source_missing")?,
            *capture_epoch,
        )?;
        reply.saved = true;
        Ok(())
    })();
    if let Err(error) = result {
        reply.code = Some(error)
    }
    reply
}
