//! 帧边界和字节预算不依赖对端自报；FD只能由启动器移交，默认在exec时关闭。
use super::protocol::{
    Binding, Envelope, Message, ProtocolError, Receiver, MAX_FRAME, MAX_MESSAGES, MAX_SESSION_BYTES,
};
use std::{
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
        unix::net::UnixStream,
    },
    time::{Duration, Instant},
};

#[derive(Debug)]
pub(super) enum TransportError {
    Io(std::io::Error),
    Protocol(ProtocolError),
    Closed,
    Timeout,
}
impl From<std::io::Error> for TransportError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}
impl From<ProtocolError> for TransportError {
    fn from(value: ProtocolError) -> Self {
        Self::Protocol(value)
    }
}
pub(super) struct Channel {
    socket: UnixStream,
    binding: Binding,
    incoming: Receiver,
    next_out: u64,
    sent_bytes: usize,
    buffer: Vec<u8>,
}
impl Channel {
    pub fn new(socket: UnixStream, binding: Binding) -> Result<Self, TransportError> {
        set_cloexec(socket.as_raw_fd())?;
        socket.set_nonblocking(true)?;
        #[cfg(target_os = "macos")]
        {
            let yes: libc::c_int = 1;
            // 断连只能成为错误，不能以SIGPIPE终止恢复执行器。
            if unsafe {
                libc::setsockopt(
                    socket.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_NOSIGPIPE,
                    (&yes as *const libc::c_int).cast(),
                    std::mem::size_of_val(&yes) as _,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        Ok(Self {
            socket,
            incoming: Receiver::new(binding.clone())?,
            binding,
            next_out: 1,
            sent_bytes: 0,
            buffer: Vec::new(),
        })
    }
    pub fn send(&mut self, message: Message) -> Result<(), TransportError> {
        let raw = serde_json::to_vec(&Envelope {
            binding: self.binding.clone(),
            sequence: self.next_out,
            message,
        })
        .map_err(|_| ProtocolError::Malformed)?;
        let next_bytes = self
            .sent_bytes
            .checked_add(raw.len())
            .ok_or(ProtocolError::Budget)?;
        if raw.len() > MAX_FRAME || next_bytes > MAX_SESSION_BYTES || self.next_out > MAX_MESSAGES {
            return Err(ProtocolError::Budget.into());
        }
        let mut frame = Vec::with_capacity(raw.len() + 4);
        frame.extend_from_slice(&(raw.len() as u32).to_be_bytes());
        frame.extend(raw);
        let deadline = Instant::now() + Duration::from_secs(1);
        let mut sent = 0;
        while sent < frame.len() {
            match self.socket.write(&frame[sent..]) {
                Ok(0) => return Err(TransportError::Closed),
                Ok(n) => sent += n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(TransportError::Timeout);
                    }
                    wait_fd(
                        self.socket.as_raw_fd(),
                        libc::POLLOUT,
                        Duration::from_millis(10),
                    )?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        }
        self.next_out += 1;
        self.sent_bytes = next_bytes;
        Ok(())
    }
    pub fn receive(&mut self, wait: Duration) -> Result<Option<(Message, bool)>, TransportError> {
        if let Some(frame) = self.take_frame()? {
            return Ok(Some(frame));
        }
        if !wait_fd(self.socket.as_raw_fd(), libc::POLLIN, wait)? {
            return Ok(None);
        }
        let mut bytes = [0u8; 4096];
        let room = (MAX_FRAME + 4)
            .saturating_sub(self.buffer.len())
            .min(bytes.len());
        if room == 0 {
            return Err(ProtocolError::Budget.into());
        }
        match self.socket.read(&mut bytes[..room]) {
            Ok(0) => Err(TransportError::Closed),
            Ok(n) => {
                self.buffer.extend_from_slice(&bytes[..n]);
                self.take_frame()
            }
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }
    fn take_frame(&mut self) -> Result<Option<(Message, bool)>, TransportError> {
        if self.buffer.len() < 4 {
            return Ok(None);
        }
        let length = u32::from_be_bytes(
            self.buffer[..4]
                .try_into()
                .map_err(|_| ProtocolError::Malformed)?,
        ) as usize;
        if length == 0 || length > MAX_FRAME {
            return Err(ProtocolError::Budget.into());
        }
        if self.buffer.len() < length + 4 {
            return Ok(None);
        }
        let result = self.incoming.accept(&self.buffer[4..length + 4])?;
        self.buffer.drain(..length + 4);
        Ok(Some(result))
    }
}
pub(super) fn set_cloexec(fd: RawFd) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}
pub(super) fn wait_fd(fd: RawFd, events: libc::c_short, wait: Duration) -> std::io::Result<bool> {
    let mut pollfd = libc::pollfd {
        fd,
        events,
        revents: 0,
    };
    let result = unsafe {
        libc::poll(
            &mut pollfd,
            1,
            wait.as_millis().min(i32::MAX as u128) as i32,
        )
    };
    if result < 0 {
        let e = std::io::Error::last_os_error();
        if e.kind() == std::io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(e);
    }
    Ok(result > 0)
}
#[cfg(target_os = "macos")]
pub(super) struct ProcessWatch {
    queue: OwnedFd,
}
#[cfg(target_os = "macos")]
impl ProcessWatch {
    /// PID只用于安装内核事件观察；调用者必须前后核同一audit实例，事件本身不授予CONT权。
    pub fn new(pid: i32) -> std::io::Result<Self> {
        let raw = unsafe { libc::kqueue() };
        if raw < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let queue = unsafe { OwnedFd::from_raw_fd(raw) };
        set_cloexec(queue.as_raw_fd())?;
        let event = libc::kevent {
            ident: pid as _,
            filter: libc::EVFILT_PROC,
            flags: libc::EV_ADD | libc::EV_ENABLE | libc::EV_CLEAR,
            fflags: libc::NOTE_EXIT | libc::NOTE_EXEC | libc::NOTE_FORK,
            data: 0,
            udata: std::ptr::null_mut(),
        };
        if unsafe {
            libc::kevent(
                queue.as_raw_fd(),
                &event,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            )
        } < 0
        {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Self { queue })
    }
    pub fn events(&self) -> std::io::Result<u32> {
        let mut event = std::mem::MaybeUninit::<libc::kevent>::uninit();
        let timeout = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let count = unsafe {
            libc::kevent(
                self.queue.as_raw_fd(),
                std::ptr::null(),
                0,
                event.as_mut_ptr(),
                1,
                &timeout,
            )
        };
        if count < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(if count == 0 {
            0
        } else {
            unsafe { event.assume_init() }.fflags
        })
    }
}
