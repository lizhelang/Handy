//! 空输入联想离线回放：逐条调用生产 `personalization::query`，只输出汇总指标。
//!
//! 用例为 JSONL：`{"set","item","context","remaining","app"?}`。命中定义为某个联想
//! 恰好是 `remaining` 的开头，不依赖评测分词与引擎词表是否一致。`--dump` 仅用于
//! 公开测试集，个人历史评测不得传入。
//!
//! 句末用例另带 `mark`（该处原文的标点）与可选 `group`（问句类型），统计问号、感叹号
//! 是否出现；表情用例带 `accept`（正例的可接受表情，反例为空）。其余用例统计句中误出标点与表情。
//!
//! 输出的 `predictions_sha256` 是全部用例联想结果（来源与文字）的摘要，用于在个人学习库副本上
//! 核对优化前后结果完全相同，而不输出任何联想文字。
use inputia_handy_runtime::personalization::{self, Query};
use inputia_handy_runtime::prediction;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
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
    #[serde(default)]
    mark: String,
    #[serde(default)]
    group: String,
    #[serde(default)]
    accept: Option<Vec<String>>,
}

const PUNCTUATION: [&str; 6] = ["，", "。", "？", "！", "；", "："];

/// 联想标点可能是半角（跟随前文），统计时统一成全角。
fn full_width(text: &str) -> String {
    match text.trim() {
        "," => "，".into(),
        "." => "。".into(),
        "?" => "？".into(),
        "!" => "！".into(),
        ";" => "；".into(),
        ":" => "：".into(),
        other => other.into(),
    }
}

fn is_punctuation(text: &str) -> bool {
    PUNCTUATION.contains(&full_width(text).as_str())
}

/// 不含汉字、字母、数字且不是标点的联想视为表情。
fn is_emoji(text: &str) -> bool {
    let text = text.trim();
    !text.is_empty()
        && !is_punctuation(text)
        && text.chars().all(|c| !c.is_alphanumeric())
        && text.chars().any(|c| c as u32 >= 0x2000)
}

/// 比较表情时忽略变体选择符，数据源之间是否带 U+FE0F 不一致。
fn bare(text: &str) -> String {
    text.trim().chars().filter(|c| *c != '\u{fe0f}').collect()
}

/// 句末某一种标点处的统计。
#[derive(Default)]
struct Ends {
    positions: u64,
    exact_first: u64,
    exact_shown: u64,
    question_shown: u64,
    exclaim_shown: u64,
    other_punct_shown: u64,
}

impl Ends {
    fn record(&mut self, predictions: &[String], mark: &str) {
        self.positions += 1;
        let predictions: Vec<String> = predictions.iter().map(|p| full_width(p)).collect();
        let shown = |target: &str| predictions.iter().any(|p| p == target);
        if predictions.first().is_some_and(|p| p == mark) {
            self.exact_first += 1;
        }
        if shown(mark) {
            self.exact_shown += 1;
        }
        if shown("？") {
            self.question_shown += 1;
        }
        if shown("！") {
            self.exclaim_shown += 1;
        }
        if predictions
            .iter()
            .any(|p| is_punctuation(p) && !matches!(p.as_str(), "？" | "！"))
        {
            self.other_punct_shown += 1;
        }
    }

    fn summary(&self) -> Value {
        let n = self.positions.max(1) as f64;
        json!({
            "positions": self.positions,
            "exact_first": round(self.exact_first as f64 / n),
            "exact_shown": round(self.exact_shown as f64 / n),
            "question_shown": round(self.question_shown as f64 / n),
            "exclaim_shown": round(self.exclaim_shown as f64 / n),
            "other_punct_shown": round(self.other_punct_shown as f64 / n),
        })
    }
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
    ends: BTreeMap<String, Ends>,
    ends_by_group: BTreeMap<String, Ends>,
    middle: u64,
    middle_punct_shown: u64,
    middle_punct_first: u64,
    middle_question_shown: u64,
    emoji_shown: u64,
    emoji_positive: u64,
    emoji_hit: u64,
    emoji_negative: u64,
    emoji_false: u64,
}

impl Tally {
    fn record(&mut self, case: &Case, predictions: &[String], micros: u64) {
        if predictions.iter().any(|p| is_emoji(p)) {
            self.emoji_shown += 1;
        }
        if let Some(accept) = &case.accept {
            let accept: Vec<String> = accept.iter().map(|e| bare(e)).collect();
            if accept.is_empty() {
                self.emoji_negative += 1;
                self.emoji_false += u64::from(predictions.iter().any(|p| is_emoji(p)));
            } else {
                self.emoji_positive += 1;
                self.emoji_hit += u64::from(predictions.iter().any(|p| accept.contains(&bare(p))));
            }
        } else if !case.mark.is_empty() {
            self.ends
                .entry(case.mark.clone())
                .or_default()
                .record(predictions, &case.mark);
            if !case.group.is_empty() {
                self.ends_by_group
                    .entry(format!("{}{}", case.group, case.mark))
                    .or_default()
                    .record(predictions, &case.mark);
            }
        } else {
            self.middle += 1;
            self.middle_punct_shown += u64::from(predictions.iter().any(|p| is_punctuation(p)));
            self.middle_punct_first +=
                u64::from(predictions.first().is_some_and(|p| is_punctuation(p)));
            self.middle_question_shown +=
                u64::from(predictions.iter().any(|p| full_width(p) == "？"));
        }
        self.record_words(predictions, &case.remaining, micros);
    }

    fn record_words(&mut self, predictions: &[String], remaining: &str, micros: u64) {
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
        let rate = |count: u64, total: u64| round(count as f64 / total.max(1) as f64);
        let mut extra = serde_json::Map::new();
        if !self.ends.is_empty() {
            extra.insert(
                "ends".into(),
                self.ends
                    .iter()
                    .map(|(k, v)| (k.clone(), v.summary()))
                    .collect(),
            );
        }
        if !self.ends_by_group.is_empty() {
            extra.insert(
                "ends_by_group".into(),
                self.ends_by_group
                    .iter()
                    .map(|(k, v)| (k.clone(), v.summary()))
                    .collect(),
            );
        }
        if self.middle > 0 {
            extra.insert(
                "middle".into(),
                json!({
                    "positions": self.middle,
                    "punct_shown": rate(self.middle_punct_shown, self.middle),
                    "punct_first": rate(self.middle_punct_first, self.middle),
                    "question_shown": rate(self.middle_question_shown, self.middle),
                }),
            );
        }
        extra.insert(
            "emoji".into(),
            json!({
                "shown_rate": rate(self.emoji_shown, self.positions),
                "positive": self.emoji_positive,
                "hit_rate": rate(self.emoji_hit, self.emoji_positive),
                "negative": self.emoji_negative,
                "false": self.emoji_false,
            }),
        );
        let mut summary = json!({
            "positions": self.positions,
            "hits": hits,
            "mrr": round(self.reciprocal_rank / n),
            "saved_chars_per_position": round(self.saved_chars as f64 / n),
            "empty_rate": round(self.empty as f64 / n),
            "all_wrong_rate": round(self.all_wrong as f64 / n),
            "shown_per_position": round(self.shown as f64 / n),
            "query_micros": {"p50": pick(0.5), "p95": pick(0.95), "max": pick(1.0)},
        });
        if let Some(object) = summary.as_object_mut() {
            object.extend(extra);
        }
        summary
    }
}

fn round(value: f64) -> f64 {
    (value * 10000.0).round() / 10000.0
}

struct Args {
    root: PathBuf,
    lexicon: Option<PathBuf>,
    ngram: Option<PathBuf>,
    backfill: Vec<String>,
    cases: PathBuf,
    limits: Vec<usize>,
    dump: Option<PathBuf>,
}

fn parse_args() -> Result<Args, String> {
    let mut root = None;
    let mut lexicon = None;
    let mut ngram = None;
    let mut backfill = Vec::new();
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
            // 按控制中心的确认回填流程导入这些来源（voice、typed）；只用于私有评测副本。
            "--backfill" => backfill = value()?.split(',').map(str::to_owned).collect(),
            "--cases" => cases = Some(PathBuf::from(value()?)),
            "--dump" => dump = Some(PathBuf::from(value()?)),
            "--limits" => {
                limits = value()?
                    .split(',')
                    .map(|v| v.trim().parse::<usize>().map_err(|e| e.to_string()))
                    .collect::<Result<_, _>>()?;
            }
            // 名字=值，覆盖默认参数；只允许在开发集上调参。
            "--tuning" => {
                let mut tuning = prediction::DEFAULT_TUNING;
                for pair in value()?.split(',') {
                    let (name, number) = pair.split_once('=').ok_or("--tuning 需要 名字=值")?;
                    let number = number.trim().parse::<f64>().map_err(|e| e.to_string())?;
                    let slot = match name.trim() {
                        "kappa" => &mut tuning.kappa,
                        "tau" => &mut tuning.tau,
                        "last_word" => &mut tuning.last_word,
                        "backoff" => &mut tuning.backoff,
                        "completion" => &mut tuning.completion,
                        "min_personal" => &mut tuning.min_personal,
                        "anchor" => &mut tuning.anchor,
                        "history" => &mut tuning.history,
                        "question" => &mut tuning.question,
                        "exclaim" => &mut tuning.exclaim,
                        "punct_tau" => &mut tuning.punct_tau,
                        "question_floor" => &mut tuning.question_floor,
                        "question_reach" => &mut tuning.question_reach,
                        "question_closing" => &mut tuning.question_closing,
                        "emoji" => &mut tuning.emoji,
                        other => return Err(format!("未知调参项 {other}")),
                    };
                    *slot = number;
                }
                prediction::set_tuning(tuning);
            }
            other => return Err(format!("未知参数 {other}")),
        }
    }
    Ok(Args {
        root: root.ok_or("需要 --root")?,
        lexicon,
        ngram,
        backfill,
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
    for source in &args.backfill {
        let started = Instant::now();
        let (mut cursor, mut imported, mut skipped) = (String::new(), 0, 0);
        loop {
            let reply = personalization::manage(
                &args.root,
                "personalization_backfill",
                &json!({"source": source, "cursor": cursor, "limit": 500}),
            )?;
            let summary = &reply["backfill_summary"];
            imported += summary["imported"].as_u64().unwrap_or(0);
            skipped += summary["skipped"].as_u64().unwrap_or(0);
            match summary["next_cursor"].as_str() {
                Some(next) if summary["has_more"] == true => cursor = next.to_owned(),
                _ => break,
            }
        }
        eprintln!(
            "backfill source={source} imported={imported} skipped={skipped} seconds={:.1}",
            started.elapsed().as_secs_f64()
        );
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
    let mut fingerprint = Sha256::new();
    for &limit in &args.limits {
        let mut tallies: BTreeMap<String, Tally> = BTreeMap::new();
        for (index, case) in cases.iter().enumerate() {
            let started = Instant::now();
            let sourced = predict(&args, epoch, case, limit)?;
            let micros = started.elapsed().as_micros() as u64;
            let predictions: Vec<String> = sourced.iter().map(|(_, text)| text.clone()).collect();
            let sources: Vec<&str> = sourced.iter().map(|(source, _)| source.as_str()).collect();
            fingerprint.update(
                json!([limit, index, sources, predictions])
                    .to_string()
                    .as_bytes(),
            );
            // 同一前文必须稳定；准入重查依赖这一点。
            if index < DETERMINISM_SAMPLE && predict(&args, epoch, case, limit)? != sourced {
                mismatches += 1;
            }
            tallies
                .entry(case.set.clone())
                .or_default()
                .record(case, &predictions, micros);
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
            "predictions_sha256": format!("{:x}", fingerprint.finalize()),
            "sets": report,
        }))
        .map_err(|e| e.to_string())?
    );
    Ok(())
}
