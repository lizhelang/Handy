use super::*;

#[test]
fn receipt_accepts_fixed_identity_and_rejects_unsafe_fields() {
    let receipt = Receipt::new(
        "owner/repo",
        "model.gguf",
        &"a".repeat(40),
        &"b".repeat(64),
        4096,
    )
    .unwrap();
    assert_eq!(
        receipt.stem(),
        format!("{}-{}-4096", "a".repeat(40), "b".repeat(64))
    );
    for (repo, filename, revision, digest, size) in [
        ("../repo", "model.gguf", "a".repeat(40), "b".repeat(64), 1),
        (
            "owner/repo",
            "../model.gguf",
            "a".repeat(40),
            "b".repeat(64),
            1,
        ),
        (
            "owner/repo",
            "model.gguf",
            "A".repeat(40),
            "b".repeat(64),
            1,
        ),
        (
            "owner/repo",
            "model.gguf",
            "a".repeat(40),
            "b".repeat(64),
            0,
        ),
    ] {
        assert!(Receipt::new(&repo, &filename, &revision, &digest, size).is_err());
    }
}

#[test]
fn url_is_bound_to_repo_revision_and_filename() {
    let receipt = Receipt::new(
        "owner/repo",
        "nested/model.gguf",
        &"a".repeat(40),
        &"b".repeat(64),
        1,
    )
    .unwrap();
    assert_eq!(
        receipt.url("https://huggingface.co/").unwrap(),
        "https://huggingface.co/owner/repo/resolve/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/nested/model.gguf"
    );
    assert!(receipt.url("https://huggingface.co/?token=secret").is_err());
}

#[test]
fn generation_and_temporary_names_are_strict() {
    assert!(generation_name(&format!(
        "{}-{}-100.gguf",
        "a".repeat(40),
        "b".repeat(64)
    )));
    assert!(generation_name(&format!(
        "{}-{}-100.partial",
        "a".repeat(40),
        "b".repeat(64)
    )));
    assert!(!generation_name("a-b-100.gguf"));
    assert!(temporary_name(
        ".active-11111111-1111-4111-8111-111111111111.tmp"
    ));
    assert!(!temporary_name(".active-not-a-uuid.tmp"));
}

#[test]
fn missing_managed_entry_is_not_discovered_or_resolved() {
    let root = tempfile::tempdir().unwrap();
    let models = root.path().join("models");
    std::fs::create_dir_all(&models).unwrap();
    assert!(resolve(&models, "owner/repo", "model.gguf")
        .unwrap()
        .is_none());
    assert!(discover(&models).is_empty());
}
