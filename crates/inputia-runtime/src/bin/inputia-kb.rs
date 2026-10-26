fn main() {
    std::process::exit(inputia_runtime::knowledge_cli::run(
        std::env::args().skip(1).collect(),
    ));
}
