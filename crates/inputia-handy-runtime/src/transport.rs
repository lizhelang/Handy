//! 阻塞式传输只供后台工作线程使用，禁止在输入法按键或 GUI 主线程调用。
//! 当前验证同 UID，并不提供代码签名身份认证。

#[cfg(unix)]
mod unix {
    use crate::protocol::{
        Handshake, HandshakePolicy, HandshakeReply, ProtocolError, MAX_FRAME_BYTES, PROTOCOL_MINOR,
    };
    use serde::{de::DeserializeOwned, Serialize};
    use std::{
        fs::{self, DirBuilder},
        io::{Read, Write},
        os::{
            fd::AsRawFd,
            unix::{
                fs::{DirBuilderExt, FileTypeExt, MetadataExt, PermissionsExt},
                net::{UnixListener, UnixStream},
            },
        },
        path::{Path, PathBuf},
        time::{Duration, Instant},
    };

    pub const IO_TIMEOUT: Duration = Duration::from_secs(2);
    pub type Result<T> = std::result::Result<T, ProtocolError>;

    /// 配置非阻塞描述符；下方同步 API 用 poll 保持完整操作的 2 秒时限。
    /// macOS 对端关闭后 setsockopt(timeout) 会返回 EINVAL，不能用它读取剩余帧。
    pub fn configure_stream(stream: &UnixStream) -> Result<()> {
        stream.set_nonblocking(true)?;
        Ok(())
    }

    fn remaining(deadline: Instant) -> Result<Duration> {
        deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
            .ok_or(ProtocolError::Timeout)
    }

    fn wait_ready(stream: &UnixStream, events: libc::c_short, deadline: Instant) -> Result<()> {
        loop {
            let timeout = remaining(deadline)?
                .as_millis()
                .saturating_add(1)
                .min(i32::MAX as u128) as i32;
            let mut descriptor = libc::pollfd {
                fd: stream.as_raw_fd(),
                events,
                revents: 0,
            };
            // SAFETY: 单个 pollfd 在调用期间有效；超时限制在 i32 毫秒范围。
            let result = unsafe { libc::poll(&mut descriptor, 1, timeout) };
            if result > 0 {
                return Ok(());
            }
            if result == 0 {
                return Err(ProtocolError::Timeout);
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error.into());
            }
        }
    }

    fn read_exact_until(
        stream: &mut UnixStream,
        mut bytes: &mut [u8],
        deadline: Instant,
    ) -> Result<()> {
        while !bytes.is_empty() {
            wait_ready(stream, libc::POLLIN, deadline)?;
            match stream.read(bytes) {
                Ok(0) => return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into()),
                Ok(count) => bytes = &mut bytes[count..],
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn read_until<T: DeserializeOwned>(stream: &mut UnixStream, deadline: Instant) -> Result<T> {
        configure_stream(stream)?;
        let mut header = [0_u8; 4];
        read_exact_until(stream, &mut header, deadline)?;
        let count = u32::from_be_bytes(header) as usize;
        if count > MAX_FRAME_BYTES {
            return Err(ProtocolError::FrameTooLarge);
        }
        if count == 0 {
            return Err(ProtocolError::EmptyFrame);
        }
        let mut bytes = vec![0; count];
        read_exact_until(stream, &mut bytes, deadline)?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// 读取一个大端 u32 长度前缀 JSON 帧，总时限 2 秒（含部分数据）。
    pub fn read_frame<T: DeserializeOwned>(stream: &mut UnixStream) -> Result<T> {
        read_until(stream, Instant::now() + IO_TIMEOUT)
    }

    fn write_until<T: Serialize>(
        stream: &mut UnixStream,
        value: &T,
        deadline: Instant,
    ) -> Result<()> {
        configure_stream(stream)?;
        let bytes = serde_json::to_vec(value)?;
        if bytes.len() > MAX_FRAME_BYTES {
            return Err(ProtocolError::FrameTooLarge);
        }
        let header = (bytes.len() as u32).to_be_bytes();
        for mut part in [header.as_slice(), bytes.as_slice()] {
            while !part.is_empty() {
                wait_ready(stream, libc::POLLOUT, deadline)?;
                match stream.write(part) {
                    Ok(0) => return Err(std::io::Error::from(std::io::ErrorKind::WriteZero).into()),
                    Ok(count) => part = &part[count..],
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
                    Err(error) => return Err(error.into()),
                }
            }
        }
        Ok(())
    }

    /// 写入失败后帧边界不确定，调用方必须关闭连接，不得继续写下一帧。
    pub fn write_frame<T: Serialize>(stream: &mut UnixStream, value: &T) -> Result<()> {
        write_until(stream, value, Instant::now() + IO_TIMEOUT)
    }

    fn user_id() -> u32 {
        // SAFETY: geteuid 不接收指针，无其他调用前置条件。
        unsafe { libc::geteuid() }
    }

    fn private_directory(path: &Path, create: bool) -> Result<()> {
        if create {
            match DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error.into()),
            }
        }
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.uid() != user_id() || metadata.mode() & 0o077 != 0 {
            return Err(ProtocolError::UnsafeEndpoint);
        }
        Ok(())
    }

    /// 拥有自己的端点；不删除已有端点，也不在销毁时删除被替换的文件。
    pub struct PrivateListener {
        listener: UnixListener,
        path: PathBuf,
        device: u64,
        inode: u64,
    }

    impl PrivateListener {
        /// 父目录不存在时只创建最后一级，权限 0700；端点为 0600。
        pub fn bind(path: impl AsRef<Path>) -> Result<Self> {
            let path = path.as_ref();
            private_directory(path.parent().ok_or(ProtocolError::UnsafeEndpoint)?, true)?;
            match fs::symlink_metadata(path) {
                Ok(_) => return Err(ProtocolError::UnsafeEndpoint),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
            let listener = UnixListener::bind(path)?;
            let metadata = fs::symlink_metadata(path)?;
            let owned = Self {
                listener,
                path: path.to_path_buf(),
                device: metadata.dev(),
                inode: metadata.ino(),
            };
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            Ok(owned)
        }

        /// 接受连接后立即检查同用户并配置读写时限。
        pub fn accept(&self) -> Result<UnixStream> {
            let (stream, _) = self.listener.accept()?;
            verify_peer(&stream)?;
            configure_stream(&stream)?;
            Ok(stream)
        }

        /// 后台事件循环可选择非阻塞 accept，以支持干净停止。
        pub fn set_nonblocking(&self, nonblocking: bool) -> Result<()> {
            self.listener.set_nonblocking(nonblocking)?;
            Ok(())
        }
    }

    impl Drop for PrivateListener {
        fn drop(&mut self) {
            if let Ok(metadata) = fs::symlink_metadata(&self.path) {
                if metadata.dev() == self.device
                    && metadata.ino() == self.inode
                    && metadata.file_type().is_socket()
                {
                    let _ = fs::remove_file(&self.path);
                }
            }
        }
    }

    /// 连接私有端点；同 UID 不代表可信发行构建，调用方仍需独立认证。
    pub fn connect(path: impl AsRef<Path>) -> Result<UnixStream> {
        let path = path.as_ref();
        private_directory(path.parent().ok_or(ProtocolError::UnsafeEndpoint)?, false)?;
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket()
            || metadata.uid() != user_id()
            || metadata.mode() & 0o077 != 0
        {
            return Err(ProtocolError::UnsafeEndpoint);
        }
        let stream = UnixStream::connect(path)?;
        verify_peer(&stream)?;
        configure_stream(&stream)?;
        Ok(stream)
    }

    fn verify_peer(stream: &UnixStream) -> Result<()> {
        if peer_uid(stream)? != user_id() {
            return Err(ProtocolError::PeerIdentity);
        }
        Ok(())
    }

    /// 从内核读取对端 UID。不会把文件所有者当成连接身份。
    pub fn peer_uid(stream: &UnixStream) -> Result<u32> {
        #[cfg(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly"
        ))]
        {
            let mut uid: libc::uid_t = 0;
            let mut gid: libc::gid_t = 0;
            // SAFETY: 描述符来自活跃 stream，两个输出指针均指向有效变量。
            if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
                return Err(ProtocolError::PeerIdentity);
            }
            Ok(uid)
        }
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let mut credentials = libc::ucred {
                pid: 0,
                uid: 0,
                gid: 0,
            };
            let mut size = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
            // SAFETY: 指针及长度对应 ucred，内核仅在该有效缓冲区内写入。
            let result = unsafe {
                libc::getsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_PEERCRED,
                    (&mut credentials as *mut libc::ucred).cast(),
                    &mut size,
                )
            };
            if result != 0 || size as usize != std::mem::size_of::<libc::ucred>() {
                return Err(ProtocolError::PeerIdentity);
            }
            Ok(credentials.uid)
        }
        #[cfg(not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "freebsd",
            target_os = "openbsd",
            target_os = "netbsd",
            target_os = "dragonfly",
            target_os = "linux",
            target_os = "android"
        )))]
        {
            let _ = stream;
            Err(ProtocolError::PeerIdentity)
        }
    }

    /// 服务端握手在单个 2 秒时限内完成；旧 epoch 仅获准同步策略屏障。
    pub fn server_handshake(
        stream: &mut UnixStream,
        server: &Handshake,
        policy: &HandshakePolicy,
    ) -> Result<Handshake> {
        server_handshake_checked(stream, server, policy, |_| Ok(()))
    }

    /// 在Accepted写出前绑定已认证对端的实例；签名认证仍须在调用本函数前完成。
    pub fn server_handshake_checked(
        stream: &mut UnixStream,
        server: &Handshake,
        policy: &HandshakePolicy,
        bind: impl FnOnce(&Handshake) -> Result<()>,
    ) -> Result<Handshake> {
        server.validate(policy).map_err(ProtocolError::Handshake)?;
        if server.policy_epoch != policy.current_policy_epoch {
            return Err(ProtocolError::InvalidHandshakeReply);
        }
        let deadline = Instant::now() + IO_TIMEOUT;
        let client: Handshake = read_until(stream, deadline)?;
        if let Err(reason) = client.validate(policy) {
            write_until(
                stream,
                &HandshakeReply::Rejected {
                    reason: reason.clone(),
                },
                deadline,
            )?;
            return Err(ProtocolError::Handshake(reason));
        }
        if bind(&client).is_err() {
            write_until(
                stream,
                &HandshakeReply::Rejected {
                    reason: crate::protocol::HandshakeRejection::InvalidIdentity,
                },
                deadline,
            )?;
            return Err(ProtocolError::PeerIdentity);
        }
        write_until(
            stream,
            &HandshakeReply::Accepted {
                server: server.clone(),
                // v1 当前只实现 minor 0；新增 minor 时在此加入能力协商。
                negotiated_minor: PROTOCOL_MINOR,
                require_policy_refresh: client.policy_epoch != policy.current_policy_epoch,
            },
            deadline,
        )?;
        Ok(client)
    }

    /// 返回明确的策略刷新状态；调用方必须完成刷新后才开放业务写入。
    pub fn client_handshake(stream: &mut UnixStream, client: &Handshake) -> Result<HandshakeReply> {
        client
            .validate(&HandshakePolicy {
                profile_id: client.profile_id.clone(),
                current_policy_epoch: client.policy_epoch,
            })
            .map_err(ProtocolError::Handshake)?;
        let deadline = Instant::now() + IO_TIMEOUT;
        write_until(stream, client, deadline)?;
        let reply: HandshakeReply = read_until(stream, deadline)?;
        match &reply {
            HandshakeReply::Rejected { reason } => {
                return Err(ProtocolError::Handshake(reason.clone()))
            }
            HandshakeReply::Accepted {
                server,
                negotiated_minor,
                require_policy_refresh,
            } => {
                server
                    .validate(&HandshakePolicy {
                        profile_id: client.profile_id.clone(),
                        current_policy_epoch: server.policy_epoch,
                    })
                    .map_err(ProtocolError::Handshake)?;
                if server.policy_epoch < client.policy_epoch
                    || *negotiated_minor != PROTOCOL_MINOR
                    || *require_policy_refresh != (server.policy_epoch != client.policy_epoch)
                {
                    return Err(ProtocolError::InvalidHandshakeReply);
                }
            }
        }
        Ok(reply)
    }
}

#[cfg(unix)]
pub use unix::*;
