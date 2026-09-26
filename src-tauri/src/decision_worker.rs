//! MLX typed-decision sidecar 管理。
//!
//! 默认不启动 worker。只有用户/开发环境显式提供
//! `INPUTIA_DECISION_WORKER` 时才启用；任何启动、协议、模型或超时错误
//! 都返回 None，由调用方继续原有确定性路径。

use inputia_handy_runtime::decision::{self, DecisionRequest, DecisionResponse};
use log::{debug, warn};
use once_cell::sync::OnceCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;
use tauri::Manager;

struct WorkerProcess {
    child: Child,
    stdin: ChildStdin,
    responses: mpsc::Receiver<Result<String, String>>,
}

impl Drop for WorkerProcess {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const MAX_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
const COLD_START_ALLOWANCE_MS: u64 = 3_000;

fn read_worker_response(reader: &mut impl BufRead) -> Result<String, String> {
    let mut line = String::new();
    let size = Read::by_ref(reader)
        .take(MAX_RESPONSE_BYTES + 1)
        .read_line(&mut line)
        .map_err(|_| "decision_worker_read".to_string())?;
    if size == 0 || size as u64 > MAX_RESPONSE_BYTES || !line.ends_with('\n') {
        return Err("decision_worker_response_bound".into());
    }
    Ok(line)
}

pub(crate) struct DecisionWorker {
    command: PathBuf,
    args: Vec<String>,
    model_id: String,
    process: Mutex<Option<WorkerProcess>>,
}

static WORKER: OnceCell<Option<Arc<DecisionWorker>>> = OnceCell::new();

fn encode_worker_request(request: &DecisionRequest) -> Result<Vec<u8>, serde_json::Error> {
    #[derive(serde::Serialize)]
    struct WorkerRequest<'a> {
        kind: &'static str,
        #[serde(flatten)]
        request: &'a DecisionRequest,
    }
    serde_json::to_vec(&WorkerRequest {
        kind: "decide",
        request,
    })
}

impl DecisionWorker {
    pub(crate) fn configure_from_resources(app: &tauri::AppHandle) {
        if std::env::var_os("INPUTIA_DECISION_WORKER").is_some() {
            return;
        }
        let Ok(resources) = app.path().resource_dir() else {
            return;
        };
        let roots = [
            resources.join("local-decision"),
            resources.join("resources/local-decision"),
        ];
        let Some(root) = roots.into_iter().find(|root| root.is_dir()) else {
            return;
        };
        let worker = root.join("inputia-decision-worker/inputia-decision-worker");
        let model = root.join("laya-multilingual-mlx");
        if worker.is_file() && model.is_dir() {
            // These values are set only for this process and are never persisted
            // to user settings or exposed to WebView JavaScript.
            std::env::set_var("INPUTIA_DECISION_WORKER", worker);
            std::env::set_var("INPUTIA_DECISION_MODEL_PATH", model);
            std::env::set_var("INPUTIA_DECISION_MODEL", "laya-multilingual-mlx");
        }
    }

    pub(crate) fn configured() -> Option<Arc<Self>> {
        WORKER
            .get_or_init(|| {
                let command = std::env::var_os("INPUTIA_DECISION_WORKER")?;
                let args = std::env::var("INPUTIA_DECISION_WORKER_ARGS")
                    .ok()
                    .map(|value| value.split_whitespace().map(str::to_owned).collect())
                    .unwrap_or_default();
                Some(Arc::new(Self {
                    command: command.into(),
                    args,
                    model_id: std::env::var("INPUTIA_DECISION_MODEL")
                        .unwrap_or_else(|_| "laya-multilingual-mlx".into()),
                    process: Mutex::new(None),
                }))
            })
            .clone()
    }

    pub(crate) fn request(&self, mut request: DecisionRequest) -> Result<DecisionResponse, String> {
        request.model_id = self.model_id.clone();
        let encoded = encode_worker_request(&request).map_err(|_| "decision_encode".to_string())?;
        decision::validate_request(&request, encoded.len())?;

        let mut guard = self
            .process
            .lock()
            .map_err(|_| "decision_lock".to_string())?;
        let cold_start = guard.is_none();
        let started = std::time::Instant::now();
        if cold_start {
            *guard = Some(self.spawn()?);
        }
        let worker = guard.as_mut().expect("worker initialized above");
        let line = encoded;
        if worker.stdin.write_all(&line).is_err() || worker.stdin.write_all(b"\n").is_err() {
            *guard = None;
            return Err("decision_worker_write".into());
        }
        if worker.stdin.flush().is_err() {
            *guard = None;
            return Err("decision_worker_flush".into());
        }
        let response_line = match worker.responses.recv_timeout(Duration::from_millis(
            request.limits.deadline_ms
                + if cold_start {
                    COLD_START_ALLOWANCE_MS
                } else {
                    0
                },
        )) {
            Ok(Ok(line)) => line,
            result => {
                let error = match result {
                    Ok(Err(error)) => error,
                    Err(mpsc::RecvTimeoutError::Timeout) => "decision_worker_timeout".into(),
                    _ => "decision_worker_read".into(),
                };
                *guard = None;
                return Err(error);
            }
        };
        let value: serde_json::Value = serde_json::from_str(&response_line)
            .map_err(|_| "decision_response_json".to_string())?;
        if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
            return Err(error.to_owned());
        }
        let response: DecisionResponse =
            serde_json::from_value(value).map_err(|_| "decision_response_shape".to_string())?;
        decision::validate_response(&request, &response)?;
        // Cold loading may finish outside the business deadline. Keep the
        // warmed process, but never apply that late result to the request.
        if response.elapsed_ms > request.limits.deadline_ms
            || started.elapsed() > Duration::from_millis(request.limits.deadline_ms)
        {
            return Err("decision_deadline".into());
        }
        Ok(response)
    }

    pub(crate) fn stop(&self) {
        if let Ok(mut guard) = self.process.lock() {
            if let Some(mut process) = guard.take() {
                let _ = process.child.kill();
                let _ = process.child.wait();
            }
        }
    }

    fn spawn(&self) -> Result<WorkerProcess, String> {
        let mut command = Command::new(&self.command);
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command
            .spawn()
            .map_err(|_| "decision_worker_spawn".to_string())?;
        let stdin = child.stdin.take().ok_or("decision_worker_stdin")?;
        let stdout = child.stdout.take().ok_or("decision_worker_stdout")?;
        let (sender, responses) = mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stdout);
            loop {
                let response = read_worker_response(&mut reader);
                let failed = response.is_err();
                if sender.send(response).is_err() || failed {
                    break;
                }
            }
        });
        debug!("local decision worker started model={}", self.model_id);
        Ok(WorkerProcess {
            child,
            stdin,
            responses,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_fixture() -> DecisionRequest {
        serde_json::from_value(serde_json::json!({
            "protocol_version": 1,
            "request_id": "worker-timeout-regression",
            "model_id": "laya-multilingual-mlx",
            "state": {"text": "测试"},
            "questions": {"route": {
                "type": "choice", "instructions": "Choose a route",
                "criteria": {"keep": "Keep", "change": "Change"}
            }},
            "limits": {"deadline_ms": 50, "max_input_chars": 4000}
        }))
        .unwrap()
    }

    #[test]
    fn worker_wire_includes_decide_kind_without_changing_domain_request() {
        let request = DecisionRequest {
            protocol_version: decision::PROTOCOL_VERSION,
            request_id: "worker-wire-regression".into(),
            model_id: "laya-multilingual-mlx".into(),
            state: serde_json::json!({"text": "测试"}),
            questions: Default::default(),
            limits: Default::default(),
        };
        let mut wire: serde_json::Value =
            serde_json::from_slice(&encode_worker_request(&request).unwrap()).unwrap();
        assert_eq!(wire["kind"], "decide");
        wire.as_object_mut().unwrap().remove("kind");
        assert_eq!(wire, serde_json::to_value(request).unwrap());
    }

    #[test]
    fn stalled_and_partial_responses_timeout_and_retire_process() {
        for output in ["", "{partial"] {
            let worker = DecisionWorker {
                command: "/usr/bin/python3".into(),
                args: vec![
                    "-c".into(),
                    format!(
                        "import sys,time; sys.stdin.readline(); sys.stdout.write({output:?}); sys.stdout.flush(); time.sleep(10)"
                    ),
                ],
                model_id: "laya-multilingual-mlx".into(),
                process: Mutex::new(None),
            };
            for _ in 0..2 {
                let started = std::time::Instant::now();
                assert_eq!(
                    worker.request(request_fixture()).unwrap_err(),
                    "decision_worker_timeout"
                );
                assert!(started.elapsed() < Duration::from_secs(5));
                assert!(worker.process.lock().unwrap().is_none());
            }
        }
    }

    #[test]
    fn response_reader_rejects_oversized_and_unterminated_lines() {
        assert!(read_worker_response(&mut &b"partial"[..]).is_err());
        assert_eq!(read_worker_response(&mut &b"{}\n"[..]).unwrap(), "{}\n");
        let oversized = vec![b'x'; MAX_RESPONSE_BYTES as usize + 1];
        assert!(read_worker_response(&mut &oversized[..]).is_err());
    }

    #[test]
    fn cold_start_can_warm_but_next_request_has_strict_deadline() {
        let script = r#"import sys,json,time
for line in sys.stdin:
    req=json.loads(line)
    time.sleep(0.2)
    print(json.dumps({"protocol_version":1,"request_id":req["request_id"],"model_id":req["model_id"],"elapsed_ms":200,"answers":{"route":{"type":"choice","selected":"keep","probabilities":{"keep":0.8,"change":0.2},"confidence":0.8,"abstained":False}}}),flush=True)
"#;
        let worker = DecisionWorker {
            command: "/usr/bin/python3".into(),
            args: vec!["-c".into(), script.into()],
            model_id: "laya-multilingual-mlx".into(),
            process: Mutex::new(None),
        };
        assert_eq!(
            worker.request(request_fixture()).unwrap_err(),
            "decision_deadline"
        );
        assert!(worker.process.lock().unwrap().is_some());
        let started = std::time::Instant::now();
        assert_eq!(
            worker.request(request_fixture()).unwrap_err(),
            "decision_worker_timeout"
        );
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(worker.process.lock().unwrap().is_none());
    }
}

pub(crate) fn request(request: DecisionRequest) -> Option<DecisionResponse> {
    let worker = DecisionWorker::configured()?;
    match worker.request(request) {
        Ok(response) => Some(response),
        Err(error) => {
            warn!("local decision unavailable: {error}");
            if error != "decision_deadline" {
                worker.stop();
            }
            None
        }
    }
}
