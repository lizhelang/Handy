//! 固定合成回放：训练语境与评估语境分离，不读取用户资料，不冒充用户命中率。
use inputia_handy_runtime::personalization::{feedback, manage, policy, query, Feedback, Query};
use serde_json::{json, Value};
use std::{path::Path, time::Instant};

struct PhraseCase {
    code: &'static str,
    pool: &'static [&'static str],
    training: &'static str,
    expected: &'static str,
    evaluation: &'static [&'static str],
}

const CASES: &[PhraseCase] = &[
    PhraseCase {
        code: "heli",
        pool: &["合理", "合力", "河里", "盒里"],
        training: "项目设计方案",
        expected: "合理",
        evaluation: &["项目的设计方案经过讨论后很", "检查设计方案是否"],
    },
    PhraseCase {
        code: "heli",
        pool: &["合理", "合力", "河里", "盒里"],
        training: "公园鱼儿游进",
        expected: "河里",
        evaluation: &["公园里面的鱼儿再次游进了", "这些鱼儿一直生活在"],
    },
    PhraseCase {
        code: "heli",
        pool: &["合理", "合力", "河里", "盒里"],
        training: "礼物包装放在",
        expected: "盒里",
        evaluation: &["礼物重新包装后放回", "把包装好的礼物收进"],
    },
    PhraseCase {
        code: "shishi",
        pool: &["实施", "事实", "试试", "实时"],
        training: "项目计划准备",
        expected: "实施",
        evaluation: &["项目计划讨论完成后正式", "新的计划即将开始"],
    },
    PhraseCase {
        code: "shishi",
        pool: &["实施", "事实", "试试", "实时"],
        training: "警方调查证据证明",
        expected: "事实",
        evaluation: &["警方整理证据后确认了", "调查发现以下"],
    },
    PhraseCase {
        code: "shishi",
        pool: &["实施", "事实", "试试", "实时"],
        training: "遇到问题不妨",
        expected: "试试",
        evaluation: &["这个问题暂时没有答案不妨再", "遇到同样的问题可以"],
    },
    PhraseCase {
        code: "gongshi",
        pool: &["共识", "公式", "工事", "工时"],
        training: "数学推导",
        expected: "公式",
        evaluation: &["数学课需要推导以下", "把数学笔记里的内容整理成"],
    },
    PhraseCase {
        code: "gongshi",
        pool: &["共识", "公式", "工事", "工时"],
        training: "讨论协商达成",
        expected: "共识",
        evaluation: &["经过讨论协商最终形成", "讨论之后大家终于达成"],
    },
    PhraseCase {
        code: "gongshi",
        pool: &["共识", "公式", "工事", "工时"],
        training: "部队修筑防御",
        expected: "工事",
        evaluation: &["部队正在加固防御", "沿线继续修筑新的"],
    },
    PhraseCase {
        code: "quanli",
        pool: &["权力", "权利", "全力"],
        training: "公民依法享有",
        expected: "权利",
        evaluation: &["每个公民都应依法享有这些", "保障公民应有的"],
    },
    PhraseCase {
        code: "quanli",
        pool: &["权力", "权利", "全力"],
        training: "监督行政机关",
        expected: "权力",
        evaluation: &["对行政机关进行监督并约束其", "行政机关依法行使"],
    },
    PhraseCase {
        code: "quanli",
        pool: &["权力", "权利", "全力"],
        training: "比赛拼搏付出",
        expected: "全力",
        evaluation: &["为了比赛继续拼搏并付出", "决赛需要顽强拼搏竭尽"],
    },
];

fn request(root: &Path, case: &PhraseCase, context: &str) -> Query {
    serde_json::from_value(json!({
        "schema_id":"luna_pinyin_simp", "input_code":case.code,
        "context":context, "context_id":"fixture-field", "source_app":"com.example.editor",
        "learning_epoch":policy(root).unwrap().epoch, "limit":5,
        "candidates":case.pool.iter().enumerate().map(|(rank,text)| json!({
            "id":format!("fixture:{rank}"),"text":text,"base_rank":rank,
            "consumed_len":case.code.len(),"match_type":"exact"
        })).collect::<Vec<_>>()
    }))
    .unwrap()
}

#[test]
fn sentence_context_replay_reports_quality_and_query_latency() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut training_count = 0;
    for (case_index, case) in CASES.iter().enumerate() {
        for repetition in 0..6 {
            let event: Feedback = serde_json::from_value(json!({
                "event_id":format!("train:{case_index}:{repetition}"),"context_id":"fixture-field",
                "schema_id":"luna_pinyin_simp","input_code":case.code,"text":case.expected,
                "previous":case.training,"explicit_selection":true,
                "original_rank":case.pool.iter().position(|text| *text == case.expected).unwrap(),
                "source_app":"com.example.editor","learning_epoch":policy(root).unwrap().epoch,
                "operation":"accept"
            }))
            .unwrap();
            feedback(root, event).unwrap();
            training_count += 1;
        }
    }

    // 评估阶段从此处开始，之后不再反馈任何测试答案。
    let before =
        manage(root, "personalization_status", &json!({})).unwrap()["counts"]["events"].clone();
    let mut baseline_top1 = 0;
    let mut baseline_top3 = 0;
    let mut learned_top1 = 0;
    let mut learned_top3 = 0;
    let mut observations = Vec::new();
    let mut latencies = Vec::new();
    for case in CASES {
        for context in case.evaluation {
            let q = request(root, case, context);
            let expected_id = q
                .candidates
                .iter()
                .find(|c| c.text == case.expected)
                .unwrap()
                .id
                .clone();
            let native_rank = q
                .candidates
                .iter()
                .position(|c| c.id == expected_id)
                .unwrap();
            let start = Instant::now();
            let result = query(root, q).unwrap();
            latencies.push(start.elapsed().as_secs_f64() * 1000.0);
            let actual_rank = result["ordered_ids"]
                .as_array()
                .unwrap()
                .iter()
                .position(|id| id == &expected_id)
                .unwrap();
            baseline_top1 += usize::from(native_rank == 0);
            baseline_top3 += usize::from(native_rank < 3);
            learned_top1 += usize::from(actual_rank == 0);
            learned_top3 += usize::from(actual_rank < 3);
            observations.push(json!({"code":case.code,"context":context,"expected":case.expected,"native_rank":native_rank+1,"rank":actual_rank+1}));
        }
    }
    assert_eq!(before, training_count);
    assert_eq!(
        manage(root, "personalization_status", &json!({})).unwrap()["counts"]["events"],
        before
    );
    latencies.sort_by(f64::total_cmp);
    let report = json!({
        "scope":"synthetic fixed pools; no user data; no live Rime; query timing excludes IMK and IPC",
        "train_events":training_count,"cases":observations.len(),
        "baseline":{"top1":baseline_top1,"top3":baseline_top3},
        "personalized":{"top1":learned_top1,"top3":learned_top3},
        "query_ms":{"p50":latencies[latencies.len()/2],"p95":latencies[(latencies.len()*95/100).min(latencies.len()-1)]},
        "observations":observations
    });
    eprintln!("CANDIDATE_QUALITY_REPLAY={report}");
    assert!(
        learned_top1 > baseline_top1,
        "改写语境首选应优于原始池：{report}"
    );
    assert!(
        learned_top3 >= baseline_top3,
        "前三候选不能整体退化：{report}"
    );
}

#[test]
fn unavailable_or_foreign_schema_evidence_never_invents_a_recalled_word() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let mut q = request(root, &CASES[0], "");
    q.candidates.clear();
    let cold = query(root, q.clone()).unwrap();
    assert_eq!(cold["ordered_ids"], json!([]));
    let event: Feedback = serde_json::from_value(json!({
        "event_id":"schema-guard","context_id":"fixture-field","schema_id":"double_pinyin_flypy",
        "input_code":"heli","text":"鹤栎","previous":"","explicit_selection":true,"original_rank":8,
        "source_app":"com.example.editor","learning_epoch":policy(root).unwrap().epoch,"operation":"accept"
    })).unwrap();
    feedback(root, event).unwrap();
    let result: Value = query(root, q).unwrap();
    assert_eq!(result["ordered_ids"], json!([]));
    assert_eq!(
        result["recalled_candidates"],
        json!([]),
        "同码不同方案不得召回"
    );
}
