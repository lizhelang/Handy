use inputia_core::ChineseEngine;
use inputia_rime::{RimeEngine, RimeEngineConfig};
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let shared = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or("需要指定共享 RimeData 目录")?,
    );
    // 回放始终使用临时用户目录，避免读取或改变真实用户的学习记录。
    let user = tempfile::tempdir()?;
    let engine = RimeEngine::open(
        RimeEngineConfig::squirrel_luna_pinyin_simp(user.path()).with_shared_data_dir(shared),
    )?;
    println!("backend={:?}\tuser_data=temporary", engine.backend_kind());
    for input in [
        "tainan",
        "woaini",
        "hainandao",
        "zhongguo",
        "zg",
        "nh",
        "woain",
        "zhonguo",
        "dagn",
        "hoa",
        "tain",
    ] {
        let native = engine.evaluate(input)?;
        let started = std::time::Instant::now();
        let adapted = engine.candidates(input);
        let elapsed_us = started.elapsed().as_micros();
        println!(
            "{input}\tpreedit={}\tnative={}\tadapted={}\tadapted_us={elapsed_us}",
            native.preedit,
            native
                .candidates
                .iter()
                .take(5)
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
                .join("|"),
            adapted
                .iter()
                .take(5)
                .map(|c| c.text.as_str())
                .collect::<Vec<_>>()
                .join("|")
        );
    }
    Ok(())
}
