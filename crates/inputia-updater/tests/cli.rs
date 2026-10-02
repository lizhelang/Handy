use std::process::Command;

#[test]
fn public_help_still_uses_public_cli() {
    let output = Command::new(env!("CARGO_BIN_EXE_inputia-updater"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8(output.stderr)
        .unwrap()
        .contains("用法：inputia-updater"));
}

#[test]
fn internal_guardian_mode_never_falls_through_to_public_parser() {
    let cases: &[&[&str]] = &[
        &[
            "--inputia-internal-suspension-guardian",
            "00000000-0000-4000-8000-000000000001",
        ],
        &[
            "--help",
            "--inputia-internal-suspension-guardian",
            "00000000-0000-4000-8000-000000000001",
        ],
        &[
            "--status",
            "--inputia-internal-suspension-guardian",
            "00000000-0000-4000-8000-000000000001",
            "--home",
            "/tmp",
        ],
        &[
            "unknown",
            "--inputia-internal-suspension-guardian",
            "00000000-0000-4000-8000-000000000001",
        ],
    ];
    for args in cases {
        let output = Command::new(env!("CARGO_BIN_EXE_inputia-updater"))
            .args(*args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(70), "args={args:?}");
        assert!(output.stdout.is_empty(), "args={args:?}");
        assert_eq!(
            String::from_utf8(output.stderr).unwrap(),
            "Inputia 内部恢复守护进程无法启动\n",
            "args={args:?}"
        );
    }
}
