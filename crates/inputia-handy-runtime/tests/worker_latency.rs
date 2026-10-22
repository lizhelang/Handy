//! 本机规模下“复制一条内容”后，IME 探针所需的策略代数读取延迟。
//! 规模可用 INPUTIA_BENCH_ROWS / INPUTIA_BENCH_IMAGES 调整。
use inputia_handy_runtime::service::HistoryService;
use rusqlite::{params, Connection};
use std::time::{Duration, Instant};

fn png_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[0, 0, 0, 255]).unwrap();
    }
    bytes
}

fn scale(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn fixture(root: &std::path::Path, rows: usize, images: usize) -> Connection {
    let history = Connection::open(root.join("history.db")).unwrap();
    history.execute_batch("CREATE TABLE transcription_history(id INTEGER PRIMARY KEY AUTOINCREMENT,file_name TEXT,timestamp INTEGER,saved INTEGER,title TEXT,transcription_text TEXT,post_processed_text TEXT);").unwrap();
    for id in 0..10 {
        history
            .execute(
                "INSERT INTO transcription_history(file_name,timestamp,saved,title,transcription_text) VALUES('',?1,0,NULL,'语音')",
                [id],
            )
            .unwrap();
    }
    let directory = root.join("clipboard_images");
    std::fs::create_dir_all(&directory).unwrap();
    let image = png_bytes();
    let mut clipboard = Connection::open(root.join("clipboard.db")).unwrap();
    clipboard.execute_batch("CREATE TABLE clipboard_history(id INTEGER PRIMARY KEY AUTOINCREMENT,content_type TEXT,full_text TEXT,title TEXT,is_favorite INTEGER,is_pinned INTEGER,created_at INTEGER,image_path TEXT,source_app TEXT);").unwrap();
    let tx = clipboard.transaction().unwrap();
    for row in 0..rows {
        if row < images {
            let name = format!("legacy-{row}.png");
            std::fs::write(directory.join(&name), &image).unwrap();
            tx.execute(
                "INSERT INTO clipboard_history(content_type,full_text,is_favorite,is_pinned,created_at,image_path) VALUES('image',NULL,0,0,?1,?2)",
                params![row as i64, name],
            )
            .unwrap();
        } else {
            tx.execute(
                "INSERT INTO clipboard_history(content_type,full_text,is_favorite,is_pinned,created_at) VALUES('text',?1,0,0,?2)",
                params![format!("复制内容 {row}"), row as i64],
            )
            .unwrap();
        }
    }
    tx.commit().unwrap();
    clipboard
}

/// 在观察窗口内反复读取，返回最慢一次；读取间隔模拟 IME 每 0.25 秒一次的探针。
fn slowest(read: &dyn Fn() -> u64, window: Duration) -> Duration {
    let until = Instant::now() + window;
    let mut slowest = Duration::ZERO;
    while Instant::now() < until {
        let started = Instant::now();
        read();
        slowest = slowest.max(started.elapsed());
        std::thread::sleep(Duration::from_millis(50));
    }
    slowest
}

#[test]
#[ignore = "本机规模基准，需显式运行"]
fn probe_epoch_read_stays_fast_after_clipboard_copy_at_local_scale() {
    let rows = scale("INPUTIA_BENCH_ROWS", 8_600);
    let images = scale("INPUTIA_BENCH_IMAGES", 2_850);
    let root = tempfile::tempdir().unwrap();
    let clipboard = fixture(root.path(), rows, images);
    let service =
        HistoryService::start(root.path().into(), "latency-fixture".into(), |_| {}).unwrap();
    service.synchronize().unwrap();
    // 等启动审计完成，再测稳定状态下的复制。
    std::thread::sleep(Duration::from_secs(5));
    let queued = || service.policy_epoch().unwrap();
    let probe = || service.probe_policy_epoch().unwrap();
    eprintln!(
        "baseline queued={:?}",
        slowest(&queued, Duration::from_secs(1))
    );
    let copy = |index: usize| {
        let _guard = service.begin_source_write();
        clipboard
            .execute(
                "INSERT INTO clipboard_history(content_type,full_text,is_favorite,is_pinned,created_at) VALUES('text',?1,0,0,?2)",
                params![format!("新复制 {index}"), (rows + index) as i64],
            )
            .unwrap();
    };
    // 前三次复制紧接着测排队读取（即同步+审计造成的工作线程停顿），后三次测探针。
    let mut worst_queued = Duration::ZERO;
    let mut worst_probe = Duration::ZERO;
    for index in 0..6 {
        copy(index);
        let (name, read): (&str, &dyn Fn() -> u64) = if index < 3 {
            ("queued", &queued)
        } else {
            ("probe", &probe)
        };
        let observed = slowest(read, Duration::from_millis(1_500));
        eprintln!("copy {index}: {name}={observed:?}");
        if index < 3 {
            worst_queued = worst_queued.max(observed);
        } else {
            worst_probe = worst_probe.max(observed);
        }
    }
    eprintln!("worst probe={worst_probe:?} queued={worst_queued:?}");
    // IME 要求 0.75 秒内拿到回复；探针读取不得排在内容线程的同步/审计之后。
    assert!(worst_probe < Duration::from_millis(100));
    // 审计逐行查询曾因无法走索引而退化为 O(N²)，本机规模下停顿约 1.3 秒。
    assert!(worst_queued < Duration::from_millis(750));
}

#[test]
fn probe_epoch_matches_authoritative_epoch_across_privacy_advance() {
    let root = tempfile::tempdir().unwrap();
    let _clipboard = fixture(root.path(), 3, 1);
    let service =
        HistoryService::start(root.path().into(), "mirror-fixture".into(), |_| {}).unwrap();
    service.synchronize().unwrap();
    let before = service.policy_epoch().unwrap();
    assert_eq!(service.probe_policy_epoch().unwrap(), before);
    let after = service
        .forget_term("forget-mirror".into(), "复制内容".into(), before)
        .unwrap();
    assert!(after > before);
    // 遗忘返回时镜像已按提交后的库内值发布，探针不能再读到被撤销的旧代数。
    assert_eq!(service.probe_policy_epoch().unwrap(), after);
    assert_eq!(service.policy_epoch().unwrap(), after);
}
