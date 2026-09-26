//! 可选本地 embedding sidecar；失败时由知识库继续使用关键词结果。
use log::warn;
use once_cell::sync::OnceCell;
use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

struct Process {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct Reply {
    pub model_id: String,
    pub model_revision: String,
    pub dimensions: usize,
    pub vectors: Vec<Vec<f32>>,
}

pub(crate) struct Worker {
    command: PathBuf,
    args: Vec<String>,
    process: Mutex<Option<Process>>,
}

static INSTANCE: OnceCell<Option<Arc<Worker>>> = OnceCell::new();

impl Worker {
    pub(crate) fn configured() -> Option<Arc<Self>> {
        INSTANCE
            .get_or_init(|| {
                Some(Arc::new(Self {
                    command: std::env::var_os("INPUTIA_EMBEDDING_WORKER")?.into(),
                    args: std::env::var("INPUTIA_EMBEDDING_WORKER_ARGS")
                        .ok()
                        .map(|value| value.split_whitespace().map(str::to_owned).collect())
                        .unwrap_or_default(),
                    process: Mutex::new(None),
                }))
            })
            .clone()
    }

    pub(crate) fn embed(&self, texts: &[String]) -> Result<Reply, String> {
        if texts.is_empty() || texts.len() > 32 {
            return Err("embedding_batch".into());
        }
        let request = serde_json::json!({
            "protocol_version": 1,
            "request_id": format!("embedding-{}", texts.len()),
            "kind": "embed",
            "texts": texts,
        });
        let mut guard = self.process.lock().map_err(|_| "embedding_lock")?;
        if guard.is_none() {
            *guard = Some(self.spawn()?);
        }
        let process = guard.as_mut().expect("embedding worker initialized");
        let mut line = serde_json::to_vec(&request).map_err(|_| "embedding_encode")?;
        line.push(b'\n');
        process
            .stdin
            .write_all(&line)
            .and_then(|_| process.stdin.flush())
            .map_err(|_| "embedding_write")?;
        let mut response = String::new();
        process
            .stdout
            .read_line(&mut response)
            .map_err(|_| "embedding_read")?;
        let value: serde_json::Value =
            serde_json::from_str(&response).map_err(|_| "embedding_json")?;
        if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
            return Err(error.into());
        }
        let reply: Reply = serde_json::from_value(value).map_err(|_| "embedding_shape")?;
        if reply.vectors.len() != texts.len()
            || reply
                .vectors
                .iter()
                .any(|vector| vector.len() != reply.dimensions)
        {
            return Err("embedding_dimensions".into());
        }
        Ok(reply)
    }

    pub(crate) fn stop(&self) {
        if let Ok(mut guard) = self.process.lock() {
            if let Some(mut process) = guard.take() {
                let _ = process.child.kill();
                let _ = process.child.wait();
            }
        }
    }

    fn spawn(&self) -> Result<Process, String> {
        let mut command = Command::new(&self.command);
        command
            .args(&self.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = command.spawn().map_err(|_| "embedding_spawn")?;
        Ok(Process {
            stdin: child.stdin.take().ok_or("embedding_stdin")?,
            stdout: BufReader::new(child.stdout.take().ok_or("embedding_stdout")?),
            child,
        })
    }
}

pub(crate) fn embed(texts: &[String]) -> Option<Reply> {
    let worker = Worker::configured()?;
    match worker.embed(texts) {
        Ok(reply) => Some(reply),
        Err(error) => {
            warn!("local embedding unavailable: {error}");
            worker.stop();
            None
        }
    }
}
