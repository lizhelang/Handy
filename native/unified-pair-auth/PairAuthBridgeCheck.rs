//! 独立 Rust→Swift 桥实验。只使用脚本创建的临时密钥、manifest 和 socket。
#[path = "../../src-tauri/src/native_pair_auth.rs"]
mod native_pair_auth;

use native_pair_auth::{EmbeddedPairTrust, PairAuthError, PairManifest, PeerRole};
use std::io::{Read, Write};
use std::os::fd::AsFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

static PUBLIC_KEY: &[u8; 65] = include_bytes!(env!("UIPA_FIXTURE_PUBLIC_KEY"));
#[cfg(pair_host_fixture)]
const LOCAL_ROLE: PeerRole = PeerRole::Inputia;
#[cfg(not(pair_host_fixture))]
const LOCAL_ROLE: PeerRole = PeerRole::Handy;
const PEER_ROLE: PeerRole = match LOCAL_ROLE {
    PeerRole::Handy => PeerRole::Inputia,
    PeerRole::Inputia => PeerRole::Handy,
};

fn trust(public_key: &'static [u8; 65], profile: &'static str) -> EmbeddedPairTrust {
    EmbeddedPairTrust::from_build_constants(
        public_key,
        "bridge-fixture-key",
        "trial-20260905",
        profile,
        LOCAL_ROLE,
    )
}
fn manifest(path: &str) -> PairManifest {
    PairManifest::load(
        &std::fs::read(path).unwrap(),
        &trust(PUBLIC_KEY, "unified-candidate:trial-20260905"),
    )
    .expect("valid signed bridge manifest")
}
fn configure(stream: &UnixStream) {
    // Darwin accept 继承监听 socket 的 O_NONBLOCK；实验收发采用有界阻塞 I/O。
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
}
fn authenticate(stream: &UnixStream, manifest: &PairManifest) -> bool {
    let started = Instant::now();
    eprintln!("bridge_fixture_authentication_begin local_role={LOCAL_ROLE:?}");
    let result = manifest.authenticate(stream.as_fd(), PEER_ROLE);
    eprintln!(
        "bridge_fixture_authentication_end local_role={LOCAL_ROLE:?} elapsed_ms={} accepted={}",
        started.elapsed().as_millis(),
        result.is_ok()
    );
    match result {
        Ok(peer) => {
            assert_eq!(peer.role(), PEER_ROLE);
            assert!(peer.audit_token().iter().any(|value| *value != 0));
            unsafe extern "C" {
                fn geteuid() -> u32;
            }
            // SAFETY: geteuid 无参数，只读当前进程有效 UID。
            assert_eq!(peer.uid(), unsafe { geteuid() });
            true
        }
        Err(PairAuthError::PeerRejected) => false,
        Err(error) => panic!("unexpected bridge error: {error}"),
    }
}
fn main() {
    assert!(native_pair_auth::candidate_build_trust("unified-candidate:trial-20260905").unwrap().is_none());
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 4 && args[1] == "client" {
        let mut stream = UnixStream::connect(&args[2]).unwrap();
        configure(&stream);
        let accepted = authenticate(&stream, &manifest(&args[3]));
        stream.write_all(&[u8::from(accepted)]).unwrap();
        let mut reply = [0];
        stream.read_exact(&mut reply).unwrap();
        std::process::exit(if accepted && reply[0] == 1 { 0 } else { 3 });
    }
    assert!(
        args.len() == 5 && args[1] == "self-check",
        "bridge fixture arguments"
    );
    let bytes = std::fs::read(&args[2]).unwrap();
    let loaded = manifest(&args[2]);
    assert!(matches!(
        PairManifest::load(&bytes, &trust(&[4; 65], "unified-candidate:trial-20260905")),
        Err(PairAuthError::ManifestRejected)
    ));
    assert!(matches!(
        PairManifest::load(&bytes, &trust(PUBLIC_KEY, "unified-candidate:other")),
        Err(PairAuthError::ManifestRejected)
    ));
    assert!(matches!(
        PairManifest::load(
            b"{}",
            &trust(PUBLIC_KEY, "unified-candidate:trial-20260905")
        ),
        Err(PairAuthError::ManifestRejected)
    ));
    assert!(matches!(
        PairManifest::load(&[], &trust(PUBLIC_KEY, "unified-candidate:trial-20260905")),
        Err(PairAuthError::InvalidArgument)
    ));
    let file = std::fs::File::open(&args[2]).unwrap();
    assert!(matches!(
        loaded.authenticate(file.as_fd(), PEER_ROLE),
        Err(PairAuthError::PeerRejected)
    ));
    assert!(matches!(
        loaded.authenticate(file.as_fd(), LOCAL_ROLE),
        Err(PairAuthError::InvalidArgument)
    ));
    for _ in 0..50 {
        drop(manifest(&args[2]));
    }
    let root = Path::new(&args[2]).parent().unwrap();
    for (label, executable, expected) in [("strict", &args[3], true), ("weak", &args[4], false)] {
        let socket = root.join(format!("{label}.sock"));
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut child = Command::new(executable)
            .args(["client", socket.to_str().unwrap(), &args[2]])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("fixture accept: {error}");
                }
            }
        };
        configure(&stream);
        let accepted = authenticate(&stream, &loaded);
        let mut peer = [0];
        stream.read_exact(&mut peer).unwrap();
        stream.write_all(&[u8::from(accepted)]).unwrap();
        assert_eq!(accepted, expected);
        assert_eq!(peer[0], u8::from(expected));
        assert_eq!(
            child.wait().unwrap().code(),
            Some(if expected { 0 } else { 3 })
        );
        drop(stream);
        drop(listener);
        std::fs::remove_file(socket).unwrap();
    }
    println!("rust_swift_pair_bridge=pass manifest_valid=true wrong_key_rejected=true invalid_fd_rejected=true raii_load_drop=50 strict_runtime_both_directions=true allowlisted_weak_runtime_rejected=true fixture_only=true");
}
