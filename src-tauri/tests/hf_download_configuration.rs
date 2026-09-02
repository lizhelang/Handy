#[test]
fn hf_model_downloads_do_not_enable_parallel_chunk_transfers() {
    let model_manager = include_str!("../src/managers/model.rs");

    assert!(
        model_manager.contains("const ATTEMPT_STREAMS: [usize; 4] = [1, 1, 1, 1]"),
        "HF 下载的每次尝试都必须保持单流，避免生成损坏的 .sync.part 文件"
    );
}
