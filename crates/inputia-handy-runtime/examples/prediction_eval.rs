//! 空输入联想离线回放：逐条调用生产 `personalization::query`，只输出汇总指标。
//!
//! 用例为 JSONL：`{"set","item","context","remaining","app"?}`。命中定义为某个联想
//! 恰好是 `remaining` 的开头，不依赖评测分词与引擎词表是否一致。`--dump` 仅用于
//! 公开测试集，个人历史评测不得传入。
use inputia_handy_runtime::personalization::{self, Query};
use inputia_handy_runtime::prediction;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::time::Instant;

const DEFAULT_APP: &str = "com.apple.TextEdit";
const DETERMINISM_SAMPLE: usize = 200;

#[derive(Deserialize)]
struct Case {
    set: String,
    item: String,
    context: String,
    remaining: String,
    #[serde(default)]
    app: String,
}

#[derive(Default)]
struct Tally {
    positions: u64,
    hits: [u64; 5],
    reciprocal_rank: f64,
    saved_chars: u64,
    empty: u64,
    all_wrong: u64,
    shown: u64,
    micros: Vec<u64>,
}

impl Tally {
    fn record(&mut self, predictions: &[String], remaining: &str, micros: u64) {
        self.positions += 1;
        self.micros.push(micros);
        if predictions.is_empty() {
            self.empty += 1;
            return;
        }
        self.shown += predictions.len() as u64;
        let remaining = remaining.trim().to_lowercase();
        let first = predictions.iter().position(|text| {
            let text = text.trim().to_lowercase();
            !text.is_empty() && remaining.starts_with(&text)
        });
        let Some(rank) = first else {
            self.all_wrong += 1;
            return;
        };
        for slot in self.hits.iter_mut().skip(rank) {
            *slot += 1;
        }
        self.reciprocal_rank += 1.0 / (rank as f64 + 1.0);
        self.saved_chars += predictions[rank].trim().chars().count() as u64;
    }

    fn summary(&mut self, limit: usize) -> Value {
        let n = self.positions.max(1) as f64;
        self.micros.sort_unstable();
        let pick = |q: f64| -> u64 {
            if self.micros.is_empty() {
                return 0;
            }
            let index = ((self.micros.len() as f64 - 1.0) * q).round() as usize;
            self.micros[index]
        };
        let mut hits = serde_json::Map::new();
        for k in 1..=limit.min(5) {
            hits.insert(
                format!("hit_at_{k}"),
                json!(round(self.hits[k - 1] as f64 / n)),
            );
        }
        json!({
            "positions": self.positions,
            "hits": hits,
            "mrr": round(self.reciprocal_rank / n),
            "saved_chars_per_position": round(self.saved_chars as f64 / n),
            "empty_rate": round(self.empty as f64 / n),
            "all_wrong_rate": round(self.all_wrong as f64 / n),
            "shown_per_position": round(self.shown as f64 / n),
            "query_micros": {"p50": pick(0.5), "p95": pick(0.95), "max": pick(1.0)},
        })
    }
}

fn round(value: f64) -> f64 {
    (value * 10000.0).round() / 10000.0
}

struct Args {
    root: PathBuf,
    lexicon: Option<PathBuf>,
    ngram: Option<PathBuf>,
    cases: PathBuf,
    limits: Vec<usize>,
    dump: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut root = None;
    let mut lexicon = None;
    let mut ngram = None;
    let mut cases = None;
    let mut limits = vec![3, 5];
    let mut dump = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} 缺少参数"));
        match flag.as_str() {
            "--root" => root = Some(PathBuf::from(value()?)),
            "--lexicon" => lexicon = Some(PathBuf::from(value()?)),
            "--ngram" => ngram = Some(PathBuf::from(value()?)),
            "--cases" => cases = Some(PathBuf::from(value()?)),
            "--dump" => dump = Some(PathBuf::from(value()?)),
            "--limits" => {
                limits = value()?
                    .split(',')
                    .map(|v| v.trim().parse::<usize>().map_err(|e| e.to_string()))
                    .collect::<Result<_, _>>()?;
            }
            // kappa,tau,last_word,backoff,completion,min_personal,anchor；只允许在开发集上调参。
            "--tuning" => {
                let values = value()?
                    .split(',')
                    .map(|v| v.trim().parse::<f64>().map_err(|e| e.to_string()))
                    .collect::<Result<Vec<_>, _>>()?;
                let [kappa, tau, last_word, backoff, completion, min_personal, anchor] = values[..]
                else {
                    return Err(
                        "--tuning 需要 kappa,tau,last_word,backoff,completion,min_personal,anchor"
                            .into(),
                    );
                };
                prediction::set_tuning(prediction::Tuning {
                    kappa,
                    tau,
                    last_word,
                    backoff,
                    completion,
                    min_personal,
                    anchor,
                });
            }
            other => return Err(format!("未知参数 {other}")),
        }
    }
    Ok(Args {
        root: root.ok_or("需要 --root")?,
        lexicon,
        ngram,
        cases: cases.ok_or("需要 --cases")?,
        limits,
        dump,
    })
}

/// 返回 (来源, 文字)；来源取 ID 前缀 `learned` / `base`，只用于公开测试集的调试输出。
fn predict(
    args: &Args,
    epoch: u64,
    case: &Case,
    limit: usize,
) -> Result<Vec<(String, String)>, String> {
    let reply = personalization::query(
        &args.root,
        Query {
            input_code: String::new(),
            schema_id: String::new(),
            context: case.context.clone(),
            context_id: format!("eval:{}", case.item),
            source_app: if case.app.is_empty() {
                DEFAULT_APP.into()
            } else {
                case.app.clone()
            },
            learning_epoch: epoch,
            candidates: Vec::new(),
            limit,
        },
    )?;
    Ok(reply["predictions"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    let source = item["id"].as_str()?.split(':').next()?.to_owned();
                    Some((source, item["text"].as_str()?.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default())
}

fn main() -> Result<(), String> {
    let args = parse_args()?;
    std::fs::create_dir_all(&args.root).map_err(|e| e.to_string())?;
    if let Some(lexicon) = &args.lexicon {
        personalization::install_base_lexicon(&args.root, lexicon)?;
    }
    if let Some(ngram) = &args.ngram {
        personalization::install_prediction_table(&args.root, ngram)?;
    }
    let epoch = personalization::policy(&args.root)?.epoch;
    let file = std::fs::File::open(&args.cases).map_err(|e| e.to_string())?;
    let mut cases = Vec::new();
    for line in std::io::BufReader::new(file).lines() {
        let line = line.map_err(|e| e.to_string())?;
        if !line.trim().is_empty() {
            cases.push(serde_json::from_str::<Case>(&line).map_err(|e| e.to_string())?);
        }
    }
    let mut dump = match &args.dump {
        Some(path) => Some(std::fs::File::create(path).map_err(|e| e.to_string())?),
        None => None,
    };
    let mut report = BTreeMap::new();
    let mut mismatches = 0u64;
    for &limit in &args.limits {
        let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
        for (index, case) in cases.iter().enumerate() {
            let started = Instant::now();
            let sourced = predict(&args, epoch, case, limit)?;
            let micros = started.elapsed().as_micros() as u64;
            let predictions: Vec<String> = sourced.iter().map(|(_, text)| text.clone()).collect();
            let sources: Vec<&str> = sourced.iter().map(|(source, _)| source.as_str()).collect();
            // 同一前文必须稳定；准入重查依赖这一点。
            if index < DETERMINISM_SAMPLE && predict(&args, epoch, case, limit)? != sourced {
                mismatches += 1;
            }
            tallies.entry(case.set.clone()).or_default().record(
                &predictions,
                &case.remaining,
                micros,
            );
            if let Some(out) = dump.as_mut() {
                writeln!(
                    out,
                    "{}",
                    json!({"limit":limit,"set":case.set,"item":case.item,"context":case.context,"remaining":case.remaining,"predictions":predictions,"sources":sources})
                )
                .map_err(|e| e.to_string())?;
            }
        }
        for (set, mut tally) in tallies {
            report
                .entry(set)
                .or_insert_with(BTreeMap::new)
                .insert(format!("limit_{limit}"), tally.summary(limit));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "cases": cases.len(),
            "determinism_mismatches": mismatches,
            "sets": report,
        }))
        .map_err(|e| e.to_string())?
    );
    Ok(())
}
