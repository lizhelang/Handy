#[test]
fn hf_model_downloads_do_not_enable_parallel_chunk_transfers() {
    let model_manager = include_str!("../src/managers/model.rs");

    assert!(
        !model_manager.contains(".with_max_files(8)"),
        "并行 HF 分块下载会产生无法校验的缓存文件；模型下载必须保持 hf-hub 的顺序默认值"
    );
}
