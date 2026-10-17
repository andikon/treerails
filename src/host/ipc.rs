//! A current-user-only local pipe. Never opens a TCP listener.
#[cfg(windows)]
mod platform {
    use anyhow::{Context, Result, ensure};
    use std::{
        fs::{File, OpenOptions},
        os::windows::io::FromRawHandle,
        ptr,
    };
    use windows_sys::Win32::{
        Foundation::{ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE, LocalFree},
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
        Storage::FileSystem::PIPE_ACCESS_DUPLEX,
        System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
            PIPE_WAIT,
        },
    };
    fn name(endpoint: &str) -> Result<Vec<u16>> {
        ensure!(
            !endpoint.is_empty()
                && endpoint.len() <= 100
                && endpoint
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "Invalid local host endpoint"
        );
        Ok(format!("\\\\.\\pipe\\treerail-{endpoint}")
            .encode_utf16()
            .chain(Some(0))
            .collect())
    }
    fn token_user(process: windows_sys::Win32::Foundation::HANDLE) -> Result<Vec<usize>> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::{
            Security::{GetTokenInformation, TOKEN_QUERY, TokenUser},
            System::Threading::OpenProcessToken,
        };
        unsafe {
            let mut token = ptr::null_mut();
            ensure!(
                OpenProcessToken(process, TOKEN_QUERY, &mut token) != 0,
                "Read local host account: {}",
                std::io::Error::last_os_error()
            );
            let token = OwnedHandle::from_raw_handle(token);
            let mut needed = 0;
            GetTokenInformation(
                token.as_raw_handle(),
                TokenUser,
                ptr::null_mut(),
                0,
                &mut needed,
            );
            ensure!(needed > 0, "Local host account identity unavailable");
            let mut data = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
            ensure!(
                GetTokenInformation(
                    token.as_raw_handle(),
                    TokenUser,
                    data.as_mut_ptr() as _,
                    needed,
                    &mut needed
                ) != 0,
                "Read local host account identity: {}",
                std::io::Error::last_os_error()
            );
            Ok(data)
        }
    }
    fn current_user_sddl() -> Result<Vec<u16>> {
        use windows_sys::Win32::{
            Security::{Authorization::ConvertSidToStringSidW, TOKEN_USER},
            System::Threading::GetCurrentProcess,
        };
        let data = token_user(unsafe { GetCurrentProcess() })?;
        unsafe {
            let user = &*(data.as_ptr() as *const TOKEN_USER);
            let mut text = ptr::null_mut();
            ensure!(
                ConvertSidToStringSidW(user.User.Sid, &mut text) != 0,
                "Format current account SID"
            );
            let mut length = 0;
            while *text.add(length) != 0 {
                length += 1;
            }
            let sid = String::from_utf16(std::slice::from_raw_parts(text, length))?;
            LocalFree(text as _);
            Ok(format!("D:P(A;;GA;;;{sid})")
                .encode_utf16()
                .chain(Some(0))
                .collect())
        }
    }
    fn verify_server_account(file: &File) -> Result<()> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::{
            Security::{EqualSid, TOKEN_USER},
            System::{
                Pipes::GetNamedPipeServerProcessId,
                Threading::{GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
            },
        };
        unsafe {
            let mut pid = 0;
            ensure!(
                GetNamedPipeServerProcessId(file.as_raw_handle(), &mut pid) != 0,
                "Identify local host process"
            );
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            ensure!(
                !handle.is_null(),
                "Inspect local host account: {}",
                std::io::Error::last_os_error()
            );
            let process = OwnedHandle::from_raw_handle(handle);
            let server = token_user(process.as_raw_handle())?;
            let client = token_user(GetCurrentProcess())?;
            let server = &*(server.as_ptr() as *const TOKEN_USER);
            let client = &*(client.as_ptr() as *const TOKEN_USER);
            ensure!(
                EqualSid(server.User.Sid, client.User.Sid) != 0,
                "Local host endpoint belongs to another account"
            );
        }
        Ok(())
    }
    #[derive(Debug)]
    pub struct ClientProcess(std::os::windows::io::OwnedHandle);
    impl ClientProcess {
        pub fn try_clone(&self) -> std::io::Result<Self> {
            Ok(Self(self.0.try_clone()?))
        }
        pub fn is_alive(&self) -> bool {
            use std::os::windows::io::AsRawHandle;
            unsafe {
                windows_sys::Win32::System::Threading::WaitForSingleObject(
                    self.0.as_raw_handle(),
                    0,
                ) == windows_sys::Win32::Foundation::WAIT_TIMEOUT
            }
        }
    }
    pub fn client_process(pipe: &Pipe) -> Result<Option<ClientProcess>> {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        unsafe {
            let mut pid = 0;
            if windows_sys::Win32::System::Pipes::GetNamedPipeClientProcessId(
                pipe.file.as_raw_handle(),
                &mut pid,
            ) == 0
            {
                return Ok(None);
            }
            let process = windows_sys::Win32::System::Threading::OpenProcess(0x00100000, 0, pid);
            if process.is_null() {
                return Ok(None);
            }
            Ok(Some(ClientProcess(OwnedHandle::from_raw_handle(process))))
        }
    }
    pub struct Pipe {
        file: File,
        deadline: std::time::Instant,
    }
    impl Pipe {
        fn new(file: File) -> Result<Self> {
            use std::os::windows::io::AsRawHandle;
            let mode = windows_sys::Win32::System::Pipes::PIPE_NOWAIT;
            ensure!(
                unsafe {
                    windows_sys::Win32::System::Pipes::SetNamedPipeHandleState(
                        file.as_raw_handle(),
                        &mode,
                        ptr::null(),
                        ptr::null(),
                    )
                } != 0,
                "Configure bounded local pipe: {}",
                std::io::Error::last_os_error()
            );
            Ok(Self {
                file,
                deadline: std::time::Instant::now() + std::time::Duration::from_secs(5),
            })
        }
        fn retry<T>(
            &mut self,
            mut operation: impl FnMut(&mut File) -> std::io::Result<T>,
        ) -> std::io::Result<T> {
            loop {
                match operation(&mut self.file) {
                    Err(error) if error.raw_os_error() == Some(232) => {
                        if std::time::Instant::now() >= self.deadline {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::TimedOut,
                                "Local host connection exceeded deadline",
                            ));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    result => return result,
                }
            }
        }
    }
    impl std::io::Read for Pipe {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            self.retry(|file| match std::io::Read::read(file, buffer) {
                Ok(0) if !buffer.is_empty() => Err(std::io::Error::from_raw_os_error(232)),
                result => result,
            })
        }
    }
    impl std::io::Write for Pipe {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.retry(|file| std::io::Write::write(file, buffer))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    pub struct Listener {
        pipe: File,
    }
    impl Listener {
        pub fn bind(endpoint: &str) -> Result<Self> {
            let name = name(endpoint)?;
            // Grant the token's actual user SID, never a possibly shared owner group.
            let sddl = current_user_sddl()?;
            let mut descriptor = ptr::null_mut();
            unsafe {
                ensure!(
                    ConvertStringSecurityDescriptorToSecurityDescriptorW(
                        sddl.as_ptr(),
                        1,
                        &mut descriptor,
                        ptr::null_mut()
                    ) != 0,
                    "Create private pipe security: {}",
                    std::io::Error::last_os_error()
                );
                let attributes = SECURITY_ATTRIBUTES {
                    nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                    lpSecurityDescriptor: descriptor,
                    bInheritHandle: 0,
                };
                let handle = CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    255,
                    65536,
                    65536,
                    0,
                    &attributes,
                );
                LocalFree(descriptor);
                ensure!(
                    handle != INVALID_HANDLE_VALUE,
                    "Create private pipe: {}",
                    std::io::Error::last_os_error()
                );
                Ok(Self {
                    pipe: File::from_raw_handle(handle),
                })
            }
        }
        pub fn accept(self) -> Result<Pipe> {
            use std::os::windows::io::AsRawHandle;
            let connected = unsafe { ConnectNamedPipe(self.pipe.as_raw_handle(), ptr::null_mut()) };
            if connected == 0 {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
                    return Err(error).context("Accept local host client");
                }
            }
            Pipe::new(self.pipe)
        }
    }
    pub fn connect(endpoint: &str) -> Result<Pipe> {
        let name = name(endpoint)?;
        let path = String::from_utf16(&name[..name.len() - 1])?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match OpenOptions::new().read(true).write(true).open(&path) {
                Ok(file) => {
                    verify_server_account(&file)?;
                    return Pipe::new(file);
                }
                Err(error)
                    if matches!(error.raw_os_error(), Some(2 | 231))
                        && std::time::Instant::now() < deadline =>
                {
                    // Retry connection establishment only: no request byte has been sent.
                    unsafe {
                        windows_sys::Win32::System::Pipes::WaitNamedPipeW(name.as_ptr(), 10);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(2));
                }
                Err(error) => return Err(error).context("Connect to local session host"),
            }
        }
    }
}
#[cfg(unix)]
mod platform {
    use anyhow::{Result, ensure};
    use std::{
        os::unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
        },
        path::PathBuf,
    };
    fn path(endpoint: &str) -> Result<PathBuf> {
        ensure!(
            !endpoint.is_empty()
                && endpoint.len() <= 100
                && endpoint
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
            "Invalid local host endpoint"
        );
        let base = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
            anyhow::anyhow!("XDG_RUNTIME_DIR required for private local host communication")
        })?;
        let base = PathBuf::from(base);
        ensure!(
            std::fs::metadata(&base)?.permissions().mode() & 0o077 == 0,
            "Runtime directory must be private to current user"
        );
        Ok(base.join(format!("treerail-{endpoint}.sock")))
    }
    #[derive(Debug)]
    pub struct ClientProcess;
    impl ClientProcess {
        pub fn try_clone(&self) -> std::io::Result<Self> {
            Ok(Self)
        }
        pub fn is_alive(&self) -> bool {
            true
        }
    }
    pub fn client_process(_: &UnixStream) -> Result<Option<ClientProcess>> {
        Ok(None)
    }
    pub struct Listener {
        socket: UnixListener,
        path: PathBuf,
    }
    impl Listener {
        pub fn bind(endpoint: &str) -> Result<Self> {
            let path = path(endpoint)?;
            let socket = UnixListener::bind(&path)?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
            Ok(Self { socket, path })
        }
        pub fn accept(self) -> Result<UnixStream> {
            Ok(self.socket.accept()?.0)
        }
    }
    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
    pub fn connect(endpoint: &str) -> Result<UnixStream> {
        Ok(UnixStream::connect(path(endpoint)?)?)
    }
}
pub use platform::*;
