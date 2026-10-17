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
    pub struct Listener {
        pipe: File,
    }
    impl Listener {
        pub fn bind(endpoint: &str) -> Result<Self> {
            let name = name(endpoint)?;
            // OW grants only the kernel-assigned owner. No Everyone/network ACEs.
            let sddl: Vec<u16> = "D:P(A;;GA;;;OW)".encode_utf16().chain(Some(0)).collect();
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
                    PIPE_ACCESS_DUPLEX | 0x00080000,
                    PIPE_TYPE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
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
        pub fn accept(self) -> Result<File> {
            use std::os::windows::io::AsRawHandle;
            let connected = unsafe { ConnectNamedPipe(self.pipe.as_raw_handle(), ptr::null_mut()) };
            if connected == 0 {
                ensure!(
                    std::io::Error::last_os_error().raw_os_error()
                        == Some(ERROR_PIPE_CONNECTED as i32),
                    "Accept local host client: {}",
                    std::io::Error::last_os_error()
                );
            }
            Ok(self.pipe)
        }
    }
    pub fn connect(endpoint: &str) -> Result<File> {
        let name = name(endpoint)?;
        let path = String::from_utf16(&name[..name.len() - 1])?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .context("Connect to local session host")
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
