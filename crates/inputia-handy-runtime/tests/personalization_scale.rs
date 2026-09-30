//! 显式运行的大数据耗时回放；全部使用合成证据和临时数据库，不代表 IMK 延迟。
use inputia_handy_runtime::personalization::{self as model, Candidate, Feedback, Query};
use rusqlite::{params, params_from_iter, Connection};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    path::Path,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const RECORDS: usize = 20_000;
const TEMPLATES: usize = 128;
const SAMPLES: usize = 20;

fn clause(index: usize) -> String {
    let topics = [
        "输入方式",
        "语音识别",
        "文本编辑",
        "候选排序",
        "拼写纠错",
        "项目设置",
        "用户词库",
        "安装流程",
        "界面布局",
        "快捷按键",
        "语言模型",
        "测试数据",
        "结果缓存",
        "消息传递",
        "资源打包",
        "应用切换",
    ];
    let stages = [
        "功能验证",
        "代码审查",
        "文档整理",
        "错误排查",
        "性能检查",
        "回归测试",
        "版本发布",
        "需求讨论",
    ];
    format!("项目团队根据真实测试反馈审查{}的设计方案随后安排{}并继续检查上下文候选排序结果是否符合预期", topics[index % topics.len()], stages[index / topics.len()])
}

fn term(index: usize) -> String {
    format!("合成候选词条{:03}", index % 256)
}

fn event(index: usize, epoch: u64) -> Feedback {
    Feedback {
        event_id: format!("scale-event-{index}"),
        context_id: "scale-field".into(),
        input_code: if index.is_multiple_of(8) {
            "uiui"
        } else {
            "shishi"
        }
        .into(),
        schema_id: if index.is_multiple_of(8) {
            "double_pinyin_flypy"
        } else {
            "luna_pinyin_simp"
        }
        .into(),
        text: term(index),
        // 每条原始语境不同；最近分句来自 128 个生产反馈模板。前句在真实
        // context_anchors 的最后分句边界外，因此复制模板锚点是准确的。
        previous: format!("合成记录{index}。{}", clause(index % TEMPLATES)),
        explicit_selection: true,
        original_rank: 1,
        source_app: if index.is_multiple_of(2) {
            "com.example.editor"
        } else {
            "com.example.chat"
        }
        .into(),
        learning_epoch: epoch,
        operation: "accept".into(),
    }
}

fn populate(root: &Path) -> (u64, usize, usize, usize) {
    let epoch = model::policy(root).unwrap().epoch;
    // 先让真实实现创建 schema、索引、字段和最终版本的锚点，不复制私有算法。
    for index in 0..TEMPLATES {
        model::feedback(root, event(index, epoch)).unwrap();
    }
    let mut db = Connection::open(root.join("knowledge/personalization.sqlite")).unwrap();
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64();
    let tx = db.transaction().unwrap();
    {
        let mut evidence = tx.prepare("INSERT INTO evidence(event_id,fingerprint,text,normalized,previous,code,weight,created,source_app,schema_id) VALUES(?1,?2,?3,?4,?5,?6,2.5,?7,?8,?9)").unwrap();
        let mut anchors = tx.prepare("INSERT INTO evidence_anchors(event_id,anchor) SELECT ?1,anchor FROM evidence_anchors WHERE event_id=?2").unwrap();
        let mut bindings = tx
            .prepare("INSERT INTO feedback_bindings(event_id,binding) VALUES(?1,?2)")
            .unwrap();
        for index in TEMPLATES..RECORDS {
            let f = event(index, epoch);
            let fingerprint = format!("{:x}", Sha256::digest(serde_json::to_vec(&f).unwrap()));
            let binding_fields = json!([
                f.context_id,
                f.input_code,
                f.text,
                f.previous,
                f.source_app,
                f.learning_epoch,
                f.schema_id
            ]);
            let binding = format!(
                "{:x}",
                Sha256::digest(binding_fields.to_string().as_bytes())
            );
            evidence
                .execute(params![
                    f.event_id,
                    fingerprint,
                    f.text,
                    f.text.to_lowercase(),
                    f.previous.to_lowercase(),
                    f.input_code,
                    timestamp - (RECORDS - index) as f64,
                    f.source_app,
                    f.schema_id
                ])
                .unwrap();
            anchors
                .execute(params![
                    f.event_id,
                    format!("scale-event-{}", index % TEMPLATES)
                ])
                .unwrap();
            bindings.execute(params![f.event_id, binding]).unwrap();
        }
        // 公共词库也具有真实主键索引和数据，避免只测空 base_phrases。
        let mut base = tx
            .prepare("INSERT INTO base_phrases(normalized,text,frequency) VALUES(?1,?1,?2)")
            .unwrap();
        for index in 0..20_000 {
            let phrase = if index < 256 {
                format!("测试反馈{}", term(index))
            } else {
                format!("公共合成搭配{index}")
            };
            base.execute(params![phrase, (20_000 - index) as f64])
                .unwrap();
        }
    }
    tx.commit().unwrap();
    let (count, distinct): (usize, usize) = db
        .query_row(
            "SELECT count(*),count(DISTINCT previous) FROM evidence",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((count, distinct), (RECORDS, RECORDS));
    let (min_anchors,max_anchors,total_anchors): (usize,usize,usize) = db.query_row("SELECT min(n),max(n),sum(n) FROM (SELECT count(*) n FROM evidence_anchors GROUP BY event_id)", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert!(min_anchors > 32 && max_anchors <= 144);
    let indexed_events: usize = db
        .query_row(
            "SELECT count(DISTINCT event_id) FROM evidence_anchors",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(indexed_events, RECORDS);
    // 额外向生产 feedback 送入一个新的前句，核对复制锚点没有漏掉语境差异。
    let check = event(RECORDS, epoch);
    model::feedback(root, check.clone()).unwrap();
    let difference: usize = db.query_row("SELECT count(*) FROM (SELECT anchor FROM evidence_anchors WHERE event_id=?1 EXCEPT SELECT anchor FROM evidence_anchors WHERE event_id=?2)", params![check.event_id,format!("scale-event-{}", RECORDS % TEMPLATES)], |r| r.get(0)).unwrap();
    assert_eq!(difference, 0);
    db.execute(
        "DELETE FROM evidence_anchors WHERE event_id=?1",
        [&check.event_id],
    )
    .unwrap();
    db.execute(
        "DELETE FROM feedback_bindings WHERE event_id=?1",
        [&check.event_id],
    )
    .unwrap();
    db.execute("DELETE FROM evidence WHERE event_id=?1", [&check.event_id])
        .unwrap();
    (epoch, min_anchors, max_anchors, total_anchors)
}

fn request(epoch: u64, count: usize, contextual: bool) -> Query {
    Query {
        input_code: "shishi".into(),
        schema_id: "luna_pinyin_simp".into(),
        context: if contextual { clause(0) } else { String::new() },
        context_id: "scale-evaluation".into(),
        source_app: "com.example.editor".into(),
        learning_epoch: epoch,
        candidates: (0..count)
            .map(|rank| Candidate {
                id: format!("native-{rank}"),
                text: term(rank),
                base_rank: rank,
                consumed_len: 6,
                match_type: "exact".into(),
            })
            .collect(),
        limit: 10,
    }
}

// 仅用于解释查询瓶颈：与测量版本主查询等价的 SQL，不用于生成排序结果。
fn explain_evidence_query(root: &Path, q: &Query) -> (Vec<String>, usize, f64) {
    let db = Connection::open(root.join("knowledge/personalization.sqlite")).unwrap();
    let mut sql = "SELECT text,normalized,previous,code,weight,created,origin,origin_revision,source_app,schema_id FROM evidence WHERE undone=0 AND normalized NOT IN(SELECT text FROM forgotten) AND (0 OR normalized IN (".to_string();
    let mut args: Vec<rusqlite::types::Value> = q
        .candidates
        .iter()
        .map(|c| c.text.to_lowercase().into())
        .collect();
    sql.push_str(&vec!["?"; q.candidates.len()].join(","));
    sql.push(')');
    if !q.context.is_empty() {
        let mut stmt = db
            .prepare("SELECT anchor FROM evidence_anchors WHERE event_id='scale-event-0'")
            .unwrap();
        let anchors = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        sql.push_str(" OR event_id IN (SELECT event_id FROM evidence_anchors WHERE anchor IN (");
        sql.push_str(&vec!["?"; anchors.len()].join(","));
        sql.push_str("))");
        args.extend(anchors.into_iter().map(Into::into));
        let suffixes: Vec<_> = q
            .context
            .char_indices()
            .rev()
            .take(256)
            .map(|(i, _)| q.context[i..].to_owned())
            .collect();
        sql.push_str(" OR previous IN (");
        sql.push_str(&vec!["?"; suffixes.len()].join(","));
        sql.push_str(") OR (normalized>=? AND normalized<?)");
        args.extend(suffixes.into_iter().map(Into::into));
        args.push(q.context.clone().into());
        args.push(format!("{}\u{10ffff}", q.context).into());
    }
    sql.push_str(") ORDER BY (origin='') DESC,created DESC LIMIT 4000");
    let legacy_sql = sql.clone();
    sql = sql.replace("(0 OR normalized", "(normalized");
    if !q.context.is_empty() {
        sql = sql
            .replacen("FROM evidence WHERE undone", "FROM evidence INDEXED BY evidence_active_recency WHERE undone", 1)
            .replace("event_id IN (SELECT event_id FROM evidence_anchors WHERE anchor IN (", "EXISTS (SELECT 1 FROM evidence_anchors WHERE evidence_anchors.event_id=evidence.event_id AND anchor IN (");
    }
    let plan = db
        .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
        .unwrap()
        .query_map(params_from_iter(&args), |r| r.get::<_, String>(3))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    let started = Instant::now();
    let mut stmt = db.prepare(&sql).unwrap();
    let mut rows = stmt.query(params_from_iter(&args)).unwrap();
    let mut selected = Vec::new();
    while let Some(row) = rows.next().unwrap() {
        selected.push(row.get::<_, String>(2).unwrap());
    }
    let sql_ms = started.elapsed().as_secs_f64() * 1000.0;
    let legacy = db
        .prepare(&legacy_sql)
        .unwrap()
        .query_map(params_from_iter(&args), |r| r.get::<_, String>(2))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    assert_eq!(selected, legacy, "优化不得通过减少或替换4000条证据来变快");
    (plan, selected.len(), sql_ms)
}

#[test]
#[ignore = "2万条合成证据性能回放；显式 --ignored --nocapture 运行，非日常单元测试"]
fn scale_reports_real_anchor_index_and_debug_query_percentiles() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let started = Instant::now();
    let (epoch, min_anchors, max_anchors, total_anchors) = populate(root);
    let database_bytes = std::fs::metadata(root.join("knowledge/personalization.sqlite"))
        .unwrap()
        .len();
    let source_sha = format!(
        "{:x}",
        Sha256::digest(include_bytes!("../src/personalization.rs"))
    );
    println!(
        "personalization_scale_fixture={}",
        json!({"records":RECORDS,"distinct_previous":RECORDS,"recent_context_templates":TEMPLATES,"anchor_rows":total_anchors,"min_anchors":min_anchors,"max_anchors":max_anchors,"database_bytes":database_bytes,"database_mib":database_bytes as f64/1048576.0,"setup_ms":started.elapsed().as_secs_f64()*1000.0,"profile":if cfg!(debug_assertions){"debug"}else{"release"},"source_sha256":source_sha,"personal_data":false,"measures_imk":false})
    );
    for count in [32, 64] {
        for contextual in [false, true] {
            let q = request(epoch, count, contextual);
            for _ in 0..2 {
                model::query(root, q.clone()).unwrap();
            }
            let mut timings = Vec::with_capacity(SAMPLES);
            let mut limited = 0;
            for _ in 0..SAMPLES {
                let started = Instant::now();
                let result = model::query(root, q.clone()).unwrap();
                assert_eq!(result["enabled"], true);
                timings.push(started.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(result["ordered_ids"].as_array().unwrap().len(), count);
                limited += usize::from(result["evidence_limited"].as_bool().unwrap());
            }
            timings.sort_by(f64::total_cmp);
            let (plan, rows, sql_ms) = explain_evidence_query(root, &q);
            println!(
                "personalization_scale_case={}",
                json!({"candidates":count,"contextual":contextual,"samples":SAMPLES,"p50_ms":timings[SAMPLES/2],"p95_ms":timings[(SAMPLES*95).div_ceil(100)-1],"max_ms":timings[SAMPLES-1],"evidence_limited_runs":limited,"evidence_rows":rows,"sql_only_sample_ms":sql_ms,"query_plan":plan})
            );
            if !cfg!(debug_assertions) {
                assert!(
                    timings[(SAMPLES * 95).div_ceil(100) - 1] < 100.0,
                    "release合成大数据查询p95超过100ms预算"
                );
            }
            if contextual {
                assert_eq!(
                    limited, SAMPLES,
                    "常见锚点应命中4000条上限，不能把少读证据误报为性能提升"
                );
            }
        }
    }

    // 稀疏路径：48个互异汉字的全部138锚点只指向一条记录，另一个语境的
    // 锚点只指向已撤销记录。不能依靠常见锚点早停掩盖最坏情况的全表扫描。
    let mut sparse_within_budget = true;
    for (offset, undone) in [(0u32, false), (2000u32, true)] {
        let context: String = (0..48)
            .map(|index| char::from_u32(0x6500 + offset + index * 7).unwrap())
            .collect();
        let mut f = event(RECORDS + offset as usize, epoch);
        f.event_id = format!("scale-sparse-{offset}");
        f.previous = context.clone();
        f.text = format!("稀疏目标{offset}");
        f.schema_id = "luna_pinyin_simp".into();
        f.source_app = "com.example.editor".into();
        f.input_code = "rare_scale".into();
        model::feedback(root, f.clone()).unwrap();
        if undone {
            f.operation = "undo".into();
            model::feedback(root, f).unwrap();
        }
        for count in [32, 64] {
            let mut q = request(epoch, count, false);
            q.context = context.clone();
            q.input_code = "rare_scale".into();
            for (index, candidate) in q.candidates.iter_mut().enumerate() {
                candidate.text = format!("未学习的候选{index}");
            }
            model::query(root, q.clone()).unwrap();
            let mut timings = Vec::new();
            for _ in 0..5 {
                let started = Instant::now();
                let result = model::query(root, q.clone()).unwrap();
                assert_eq!(result["enabled"], true);
                timings.push(started.elapsed().as_secs_f64() * 1000.0);
                assert_eq!(result["evidence_limited"], false);
            }
            timings.sort_by(f64::total_cmp);
            println!(
                "personalization_scale_sparse={}",
                json!({"candidates":count,"only_undone_anchor":undone,"samples":5,"p50_ms":timings[2],"p95_ms":timings[4],"evidence_limited":false})
            );
            sparse_within_budget &= timings[4] < 100.0;
        }
    }
    if !cfg!(debug_assertions) {
        assert!(sparse_within_budget, "release稀疏锚点查询p95超过100ms预算");
    }
}
