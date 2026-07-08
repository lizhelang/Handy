use std::path::PathBuf;

use inputia_rime::{RimeEngine, RimeEngineConfig};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(|arg| arg.as_str()) == Some("--matrix") {
        return run_matrix(&args[1..]);
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
