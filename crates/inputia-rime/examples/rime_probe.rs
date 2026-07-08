use std::path::PathBuf;

use inputia_core::ChineseEngine;
use inputia_rime::{RimeEngine, RimeEngineConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(|arg| arg.as_str()) == Some("--matrix") {
        return run_matrix(&args[1..]);
    }
    if args.first().map(|arg| arg.as_str()) == Some("--select-matrix") {
        return run_select_matrix(&args[1..]);
    }

    let schema = args
        .first()
        .cloned()
        .unwrap_or_else(|| "luna_pinyin_simp".to_string());
    let keys = args.get(1).cloned().unwrap_or_else(|| "ni".to_string());
    let user_data_dir = rime_user_data_dir();

    let engine = open_engine(&schema, user_data_dir)?;
    let snapshot = engine.evaluate(&keys)?;
    print_snapshot("", &snapshot, &keys);

    Ok(())
}

fn run_matrix(specs: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if specs.is_empty() {
        return Err(
            "matrix mode requires schema:keys:expected_present[:expected_first] specs".into(),
        );
    }

    let base_user_data_dir = rime_user_data_dir();
    println!("probeCount={}", specs.len());
    for (index, spec) in specs.iter().enumerate() {
        let case = ProbeCase::parse(spec)?;
        let user_data_dir = base_user_data_dir.join(format!(
            "case-{index}-{}-{}",
            sanitize_path_fragment(&case.schema),
            sanitize_path_fragment(&case.keys)
        ));
        let engine = open_engine(&case.schema, user_data_dir)?;
        let snapshot = engine.evaluate(&case.keys)?;
        let prefix = format!("probe[{index}].");
        print_snapshot(&prefix, &snapshot, &case.keys);
        assert_expected_candidates(&prefix, &snapshot, &case)?;
    }
    println!("probeMatrixSelfCheck=true");

    Ok(())
}

fn run_select_matrix(specs: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if specs.is_empty() {
        return Err(
            "select-matrix mode requires schema:keys:one_based_index:expected_selected:expected_commit:expected_preedit:expected_next_present[:expected_next_first] specs"
                .into(),
        );
    }

    let base_user_data_dir = rime_user_data_dir();
    println!("selectProbeCount={}", specs.len());
    for (index, spec) in specs.iter().enumerate() {
        let case = SelectProbeCase::parse(spec)?;
        let user_data_dir = base_user_data_dir.join(format!(
            "select-case-{index}-{}-{}",
            sanitize_path_fragment(&case.schema),
            sanitize_path_fragment(&case.keys)
        ));
        let engine = open_engine(&case.schema, user_data_dir)?;
        let candidates = engine.candidates(&case.keys);
        let prefix = format!("selectProbe[{index}].");
        print_candidate_list(&prefix, &case.keys, &case.schema, &candidates);
        assert_selected_candidate(&prefix, &candidates, &case)?;

        let selected = candidates[case.zero_based_index].clone();
        let snapshot =
            engine.select_live_candidate(&case.keys, &selected, 0, case.zero_based_index)?;
        print_selected_snapshot(&prefix, &snapshot);
        assert_selected_snapshot(&prefix, &snapshot, &case)?;
    }
    println!("selectProbeMatrixSelfCheck=true");

    Ok(())
}

fn rime_user_data_dir() -> PathBuf {
    std::env::var("INPUTIA_RIME_USER_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(format!("/tmp/inputia-rime-adapter-{}", std::process::id()))
        })
}

fn open_engine(
    schema: &str,
    user_data_dir: PathBuf,
) -> Result<RimeEngine, Box<dyn std::error::Error>> {
    let mut config = RimeEngineConfig::squirrel_luna_pinyin_simp(user_data_dir).with_schema(schema);
    if let Ok(path) = std::env::var("INPUTIA_RIME_DYLIB") {
        config = config.with_dylib_path(path);
    }
    if let Ok(path) = std::env::var("INPUTIA_RIME_SHARED_DATA_DIR") {
        config = config.with_shared_data_dir(path);
    }

    Ok(RimeEngine::open(config)?)
}

fn print_candidate_list(
    prefix: &str,
    keys: &str,
    schema: &str,
    candidates: &[inputia_core::Candidate],
) {
    println!("{prefix}schema={schema}");
    println!("{prefix}keys={keys}");
    println!("{prefix}candidateCount={}", candidates.len());
    for (index, candidate) in candidates.iter().enumerate() {
        println!(
            "{prefix}candidate[{index}]={}\t{}",
            candidate.text, candidate.annotation
        );
    }
}

fn print_snapshot(prefix: &str, snapshot: &inputia_rime::RimeSnapshot, keys: &str) {
    println!("{prefix}schema={}", snapshot.schema_id);
    println!("{prefix}keys={keys}");
    println!("{prefix}input={}", snapshot.input);
    println!("{prefix}preedit={}", snapshot.preedit);
    println!(
        "{prefix}cursor={} sel_start={} sel_end={}",
        snapshot.cursor_pos, snapshot.sel_start, snapshot.sel_end
    );
    println!(
        "{prefix}page={} page_size={} candidates={} last={} highlighted={}",
        snapshot.page_no,
        snapshot.page_size,
        snapshot.candidates.len(),
        snapshot.is_last_page,
        snapshot.highlighted_candidate_index
    );
    for (index, candidate) in snapshot.candidates.iter().enumerate() {
        println!(
            "{prefix}candidate[{index}]={}\t{}",
            candidate.text, candidate.annotation
        );
    }
    println!(
        "{prefix}commit={}",
        snapshot.commit.as_deref().unwrap_or("<none>")
    );
}

fn print_selected_snapshot(prefix: &str, snapshot: &inputia_rime::RimeSnapshot) {
    println!(
        "{prefix}selectedCommit={}",
        snapshot.commit.as_deref().unwrap_or("<none>")
    );
    println!("{prefix}selectedInput={}", snapshot.input);
    println!("{prefix}selectedPreedit={}", snapshot.preedit);
    println!(
        "{prefix}selectedPage={} selectedCandidates={} selectedLast={}",
        snapshot.page_no,
        snapshot.candidates.len(),
        snapshot.is_last_page
    );
    for (index, candidate) in snapshot.candidates.iter().enumerate() {
        println!(
            "{prefix}selectedCandidate[{index}]={}\t{}",
            candidate.text, candidate.annotation
        );
    }
}

fn assert_selected_candidate(
    prefix: &str,
    candidates: &[inputia_core::Candidate],
    case: &SelectProbeCase,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(selected) = candidates.get(case.zero_based_index) else {
        return Err(format!(
            "{}{} has no candidate at one-based index {} for {}",
            prefix, case.schema, case.one_based_index, case.keys
        )
        .into());
    };
    let selected_matches = selected.text == case.expected_selected;
    println!("{prefix}expectedSelected={}", case.expected_selected);
    println!("{prefix}selectedMatches={selected_matches}");
    if !selected_matches {
        return Err(format!(
            "{}{} candidate {} should be {} for {}",
            prefix, case.schema, case.one_based_index, case.expected_selected, case.keys
        )
        .into());
    }
    Ok(())
}

fn assert_selected_snapshot(
    prefix: &str,
    snapshot: &inputia_rime::RimeSnapshot,
    case: &SelectProbeCase,
) -> Result<(), Box<dyn std::error::Error>> {
    let actual_commit = snapshot.commit.as_deref();
    let expected_commit = case.expected_commit.as_deref();
    let commit_matches = actual_commit == expected_commit;
    println!(
        "{prefix}expectedCommit={}",
        expected_commit.unwrap_or("<none>")
    );
    println!("{prefix}selectedCommitMatches={commit_matches}");
    if !commit_matches {
        return Err(format!(
            "{}{} select should commit {:?}, got {:?}",
            prefix, case.schema, expected_commit, actual_commit
        )
        .into());
    }

    let preedit_matches = snapshot.preedit == case.expected_preedit;
    println!("{prefix}expectedPreedit={}", case.expected_preedit);
    println!("{prefix}selectedPreeditMatches={preedit_matches}");
    if !preedit_matches {
        return Err(format!(
            "{}{} select should keep preedit {:?}, got {:?}",
            prefix, case.schema, case.expected_preedit, snapshot.preedit
        )
        .into());
    }

    let expected_next_present_found = snapshot
        .candidates
        .iter()
        .any(|candidate| candidate.text == case.expected_next_present);
    println!("{prefix}expectedNextPresent={}", case.expected_next_present);
    println!("{prefix}expectedNextPresentFound={expected_next_present_found}");
    if !expected_next_present_found {
        return Err(format!(
            "{}{} select should keep next candidate {} instead of clearing composition",
            prefix, case.schema, case.expected_next_present
        )
        .into());
    }

    if let Some(expected_next_first) = &case.expected_next_first {
        let next_first = snapshot
            .candidates
            .first()
            .map(|candidate| candidate.text.as_str());
        let next_first_matches = next_first == Some(expected_next_first.as_str());
        println!("{prefix}expectedNextFirst={expected_next_first}");
        println!("{prefix}expectedNextFirstMatches={next_first_matches}");
        if !next_first_matches {
            return Err(format!(
                "{}{} select should rank {} first after partial selection",
                prefix, case.schema, expected_next_first
            )
            .into());
        }
    } else {
        println!("{prefix}expectedNextFirst=<skipped>");
        println!("{prefix}expectedNextFirstMatches=skipped");
    }

    Ok(())
}

fn assert_expected_candidates(
    prefix: &str,
    snapshot: &inputia_rime::RimeSnapshot,
    case: &ProbeCase,
) -> Result<(), Box<dyn std::error::Error>> {
    let expected_present_found = snapshot
        .candidates
        .iter()
        .any(|candidate| candidate.text == case.expected_present);
    println!("{prefix}expectedPresent={}", case.expected_present);
    println!("{prefix}expectedPresentFound={expected_present_found}");
    if !expected_present_found {
        return Err(format!(
            "{}{} should include {} instead of falling back to raw letters",
            prefix, case.schema, case.expected_present
        )
        .into());
    }

    if let Some(expected_first) = &case.expected_first {
        let first = snapshot
            .candidates
            .first()
            .map(|candidate| candidate.text.as_str());
        let expected_first_matches = first == Some(expected_first.as_str());
        println!("{prefix}expectedFirst={expected_first}");
        println!("{prefix}expectedFirstMatches={expected_first_matches}");
        if !expected_first_matches {
            return Err(format!(
                "{}{} should rank {} first for {}",
                prefix, case.schema, expected_first, case.keys
            )
            .into());
        }
    } else {
        println!("{prefix}expectedFirst=<skipped>");
        println!("{prefix}expectedFirstMatches=skipped");
    }

    Ok(())
}

fn sanitize_path_fragment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

struct ProbeCase {
    schema: String,
    keys: String,
    expected_present: String,
    expected_first: Option<String>,
}

impl ProbeCase {
    fn parse(spec: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let parts = spec.split(':').collect::<Vec<_>>();
        if !(3..=4).contains(&parts.len()) {
            return Err(format!(
                "invalid probe spec {spec:?}; expected schema:keys:expected_present[:expected_first]"
            )
            .into());
        }
        let expected_first = match parts.get(3).copied() {
            Some("") | Some("-") | None => None,
            Some(value) => Some(value.to_string()),
        };
        Ok(Self {
            schema: parts[0].to_string(),
            keys: parts[1].to_string(),
            expected_present: parts[2].to_string(),
            expected_first,
        })
    }
}

struct SelectProbeCase {
    schema: String,
    keys: String,
    one_based_index: usize,
    zero_based_index: usize,
    expected_selected: String,
    expected_commit: Option<String>,
    expected_preedit: String,
    expected_next_present: String,
    expected_next_first: Option<String>,
}

impl SelectProbeCase {
    fn parse(spec: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let parts = spec.split(':').collect::<Vec<_>>();
        if !(7..=8).contains(&parts.len()) {
            return Err(format!(
                "invalid select probe spec {spec:?}; expected schema:keys:one_based_index:expected_selected:expected_commit:expected_preedit:expected_next_present[:expected_next_first]"
            )
            .into());
        }
        let one_based_index = parts[2].parse::<usize>()?;
        if one_based_index == 0 {
            return Err("select probe index is one-based and must be >= 1".into());
        }
        let expected_commit = match parts[4] {
            "" | "-" | "<none>" => None,
            value => Some(value.to_string()),
        };
        let expected_next_first = match parts.get(7).copied() {
            Some("") | Some("-") | None => None,
            Some(value) => Some(value.to_string()),
        };
        Ok(Self {
            schema: parts[0].to_string(),
            keys: parts[1].to_string(),
            one_based_index,
            zero_based_index: one_based_index - 1,
            expected_selected: parts[3].to_string(),
            expected_commit,
            expected_preedit: parts[5].to_string(),
            expected_next_present: parts[6].to_string(),
            expected_next_first,
        })
    }
}
