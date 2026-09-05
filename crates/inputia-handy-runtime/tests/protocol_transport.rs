#![cfg(unix)]

use inputia_handy_runtime::{
    protocol::{
        ControlEnvelope, ControlVerb, Handshake, HandshakePolicy, HandshakeRejection,
        HandshakeReply, PolicyGate, ProtocolError, MAX_FRAME_BYTES,
    },
    transport::{
        client_handshake, connect, peer_uid, read_frame, server_handshake, write_frame,
        PrivateListener, IO_TIMEOUT,
    },
};
use std::{
    fs,
    io::Write,
    os::fd::AsRawFd,
    os::unix::{
        fs::{symlink, PermissionsExt},
        net::UnixStream,
    },
    thread,
    time::{Duration, Instant},
};

fn handshake(instance: &str, epoch: u64) -> Handshake {
    Handshake {
        protocol_major: 1,
        protocol_minor: 0,
        instance_id: instance.into(),
        profile_id: "synthetic-profile".into(),
        policy_epoch: epoch,
        capabilities: vec!["voice_control".into()],
    }
}

fn policy() -> HandshakePolicy {
    HandshakePolicy {
        profile_id: "synthetic-profile".into(),
        current_policy_epoch: 7,
    }
}

#[test]
fn one_hundred_real_connections_handshake_status_and_disconnect() {
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("private/control.sock");
    let listener = PrivateListener::bind(&socket).unwrap();
    assert_eq!(
        fs::metadata(socket.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let worker = thread::spawn(move || {
        for index in 0..100 {
            let mut stream = listener.accept().unwrap();
            let hello =
                server_handshake(&mut stream, &handshake("server-instance", 7), &policy()).unwrap();
            assert_eq!(hello.instance_id, format!("client-{index}"));
            let request: ControlEnvelope<()> = read_frame(&mut stream).unwrap();
            request.validate().unwrap();
            assert_eq!(request.verb, ControlVerb::Status);
            write_frame(&mut stream, &request).unwrap();
        }
    });
    let started = Instant::now();
    for index in 0..100 {
        let mut stream = connect(&socket).unwrap();
        // 证明内核返回实际同 UID；这不是代码签名验证。
        assert_eq!(peer_uid(&stream).unwrap(), unsafe { libc::geteuid() });
        let reply =
            client_handshake(&mut stream, &handshake(&format!("client-{index}"), 6)).unwrap();
        assert!(matches!(
            reply,
            HandshakeReply::Accepted {
                require_policy_refresh: true,
                ..
            }
        ));
        let request = ControlEnvelope {
            request_id: format!("request-{index}"),
            session_id: None,
            operation_id: None,
            verb: ControlVerb::Status,
            payload: (),
        };
        write_frame(&mut stream, &request).unwrap();
        let response: ControlEnvelope<()> = read_frame(&mut stream).unwrap();
        assert_eq!(response, request);
    }
    worker.join().unwrap();
    eprintln!(
        "100 real Unix handshakes/status/disconnects: {:?}",
        started.elapsed()
    );
    assert!(!socket.exists());
}

#[test]
fn refuses_wrong_version_profile_and_future_epoch_with_wire_rejection() {
    for (mut client, expected) in [
        (
            handshake("client", 7),
            HandshakeRejection::IncompatibleVersion,
        ),
        (handshake("client", 7), HandshakeRejection::ProfileMismatch),
        (
            handshake("client", 8),
            HandshakeRejection::FuturePolicyEpoch,
        ),
    ] {
        match expected {
            HandshakeRejection::IncompatibleVersion => client.protocol_major = 2,
            HandshakeRejection::ProfileMismatch => client.profile_id = "other-profile".into(),
            _ => {}
        }
        let (mut left, mut right) = UnixStream::pair().unwrap();
        let worker =
            thread::spawn(move || server_handshake(&mut right, &handshake("server", 7), &policy()));
        write_frame(&mut left, &client).unwrap();
        let reply: HandshakeReply = read_frame(&mut left).unwrap();
        assert_eq!(
            reply,
            HandshakeReply::Rejected {
                reason: expected.clone()
            }
        );
        assert!(
            matches!(worker.join().unwrap(), Err(ProtocolError::Handshake(reason)) if reason == expected)
        );
    }
}

#[test]
fn malformed_frames_are_bounded_and_fail_without_panics() {
    for (bytes, kind) in [
        (vec![0, 0], "truncated"),
        (vec![0, 0, 0, 5, b'{'], "truncated"),
        (
            ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes().to_vec(),
            "large",
        ),
        (vec![0, 0, 0, 0], "empty"),
        (vec![0, 0, 0, 1, b'{'], "json"),
    ] {
        let (mut writer, mut reader) = UnixStream::pair().unwrap();
        writer.write_all(&bytes).unwrap();
        drop(writer);
        let result = read_frame::<serde_json::Value>(&mut reader);
        match kind {
            "truncated" => assert!(
                matches!(result, Err(ProtocolError::Io(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof)
            ),
            "large" => assert!(matches!(result, Err(ProtocolError::FrameTooLarge))),
            "empty" => assert!(matches!(result, Err(ProtocolError::EmptyFrame))),
            "json" => assert!(matches!(result, Err(ProtocolError::Json(_)))),
            _ => unreachable!(),
        }
    }
    let (mut writer, _reader) = UnixStream::pair().unwrap();
    assert!(matches!(
        write_frame(&mut writer, &"x".repeat(MAX_FRAME_BYTES)),
        Err(ProtocolError::FrameTooLarge)
    ));
}

#[test]
fn timeout_applies_to_total_frame_even_when_some_bytes_arrive() {
    let (mut writer, mut reader) = UnixStream::pair().unwrap();
    let worker = thread::spawn(move || {
        writer.write_all(&[0, 0]).unwrap();
        thread::sleep(Duration::from_millis(1200));
        writer.write_all(&[0, 8, b'{']).unwrap();
        thread::sleep(Duration::from_millis(1200));
    });
    let start = Instant::now();
    assert!(matches!(
        read_frame::<Handshake>(&mut reader),
        Err(ProtocolError::Timeout)
    ));
    assert!(start.elapsed() >= IO_TIMEOUT - Duration::from_millis(100));
    assert!(start.elapsed() < Duration::from_millis(2800));
    worker.join().unwrap();
}

#[test]
fn handshake_timeout_does_not_report_ready() {
    let (mut client, _silent_peer) = UnixStream::pair().unwrap();
    assert!(matches!(
        client_handshake(&mut client, &handshake("client", 7)),
        Err(ProtocolError::Timeout)
    ));
}

#[test]
fn rejects_symlinks_existing_endpoints_and_public_directories() {
    let temp = tempfile::tempdir().unwrap();
    let private = temp.path().join("private");
    let socket = private.join("socket");
    let first = PrivateListener::bind(&socket).unwrap();
    assert!(matches!(
        PrivateListener::bind(&socket),
        Err(ProtocolError::UnsafeEndpoint)
    ));
    drop(first);
    fs::write(&socket, "user-data").unwrap();
    assert!(matches!(
        PrivateListener::bind(&socket),
        Err(ProtocolError::UnsafeEndpoint)
    ));
    assert_eq!(fs::read_to_string(&socket).unwrap(), "user-data");
    fs::remove_file(&socket).unwrap();
    symlink("missing", &socket).unwrap();
    assert!(matches!(
        PrivateListener::bind(&socket),
        Err(ProtocolError::UnsafeEndpoint)
    ));
    assert!(matches!(
        connect(&socket),
        Err(ProtocolError::UnsafeEndpoint)
    ));
    let alias = temp.path().join("alias");
    symlink(&private, &alias).unwrap();
    assert!(matches!(
        PrivateListener::bind(alias.join("new")),
        Err(ProtocolError::UnsafeEndpoint)
    ));
    fs::set_permissions(&private, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(matches!(
        PrivateListener::bind(private.join("new")),
        Err(ProtocolError::UnsafeEndpoint)
    ));
}

#[test]
fn drop_preserves_replacement_inode() {
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("private/socket");
    let listener = PrivateListener::bind(&socket).unwrap();
    fs::rename(&socket, socket.with_extension("old")).unwrap();
    fs::write(&socket, "new-data").unwrap();
    drop(listener);
    assert_eq!(fs::read_to_string(&socket).unwrap(), "new-data");
}

#[test]
fn client_rejects_server_profile_and_false_refresh_claim() {
    for wrong_profile in [true, false] {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let worker = thread::spawn(move || {
            let _: Handshake = read_frame(&mut server).unwrap();
            let mut hello = handshake("server", 7);
            if wrong_profile {
                hello.profile_id = "unrelated".into();
            }
            write_frame(
                &mut server,
                &HandshakeReply::Accepted {
                    server: hello,
                    negotiated_minor: 0,
                    require_policy_refresh: false,
                },
            )
            .unwrap();
        });
        assert!(client_handshake(&mut client, &handshake("client", 6)).is_err());
        worker.join().unwrap();
    }
}

#[test]
fn policy_barrier_blocks_writes_until_current_snapshot_acknowledged() {
    let mut gate = PolicyGate::new(7);
    gate.authorize(ControlVerb::Status).unwrap();
    gate.authorize(ControlVerb::Heartbeat).unwrap();
    assert!(gate.authorize(ControlVerb::StartVoice).is_err());
    assert!(gate.acknowledge(6).is_err());
    gate.acknowledge(7).unwrap();
    gate.authorize(ControlVerb::StartVoice).unwrap();
    gate.advance(8).unwrap();
    assert!(gate.authorize(ControlVerb::StartVoice).is_err());
    gate.authorize(ControlVerb::StopVoice).unwrap();
    gate.authorize(ControlVerb::CancelVoice).unwrap();
    assert!(gate.advance(7).is_err());
    assert!(gate.acknowledge(7).is_err());
    gate.acknowledge(8).unwrap();
    gate.authorize(ControlVerb::CancelVoice).unwrap();
}

#[test]
fn control_envelope_rejects_missing_session_and_control_character_ids() {
    let mut request = ControlEnvelope {
        request_id: "r-1".into(),
        session_id: None,
        operation_id: None,
        verb: ControlVerb::StartVoice,
        payload: (),
    };
    assert!(request.validate().is_err());
    request.session_id = Some("s-1".into());
    request.validate().unwrap();
    request.request_id = "r\n1".into();
    assert!(request.validate().is_err());
}

#[test]
fn exact_frame_limit_roundtrips_without_truncation() {
    let (mut writer, mut reader) = UnixStream::pair().unwrap();
    // JSON 字符串的双引号也计入帧长度。
    let expected = "x".repeat(MAX_FRAME_BYTES - 2);
    let sent = expected.clone();
    let worker = thread::spawn(move || write_frame(&mut writer, &sent).unwrap());
    assert_eq!(read_frame::<String>(&mut reader).unwrap(), expected);
    worker.join().unwrap();
}

#[test]
fn higher_minor_negotiates_supported_version_and_equal_epoch() {
    let (mut client, mut server) = UnixStream::pair().unwrap();
    let worker = thread::spawn(move || {
        server_handshake(&mut server, &handshake("server", 7), &policy()).unwrap();
    });
    let mut hello = handshake("client", 7);
    hello.protocol_minor = 100;
    assert!(matches!(
        client_handshake(&mut client, &hello).unwrap(),
        HandshakeReply::Accepted {
            negotiated_minor: 0,
            require_policy_refresh: false,
            ..
        }
    ));
    worker.join().unwrap();
}

#[test]
fn socket_file_permissions_are_checked_before_connection() {
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("private/socket");
    let _listener = PrivateListener::bind(&socket).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o666)).unwrap();
    assert!(matches!(
        connect(&socket),
        Err(ProtocolError::UnsafeEndpoint)
    ));
}

#[test]
fn stalled_reader_bounds_write_to_two_seconds() {
    let (mut writer, _reader) = UnixStream::pair().unwrap();
    let size: libc::c_int = 4096;
    // SAFETY: SO_SNDBUF 接收与 size 对应的有效整数缓冲区。
    let result = unsafe {
        libc::setsockopt(
            writer.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_SNDBUF,
            (&size as *const libc::c_int).cast(),
            std::mem::size_of_val(&size) as libc::socklen_t,
        )
    };
    assert_eq!(result, 0);
    let started = Instant::now();
    assert!(matches!(
        write_frame(&mut writer, &"x".repeat(MAX_FRAME_BYTES - 2)),
        Err(ProtocolError::Timeout)
    ));
    assert!(started.elapsed() >= IO_TIMEOUT - Duration::from_millis(100));
    assert!(started.elapsed() < Duration::from_millis(2800));
}
