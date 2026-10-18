#[cfg(windows)]
fn valid_identity(name: &str) -> bool {
    name.strip_prefix("Local\\TreeRail-")
        .is_some_and(|identity| {
            crate::copilot::is_uuid(identity)
                || identity.is_ascii()
                    && identity.len() == 73
                    && identity.as_bytes()[36] == b'-'
                    && crate::copilot::is_uuid(&identity[..36])
                    && crate::copilot::is_uuid(&identity[37..])
        })
}
pub fn execution_group_identity(host_instance: &str, request_id: &str) -> anyhow::Result<String> {
    anyhow::ensure!(
        crate::copilot::is_uuid(host_instance) && crate::copilot::is_uuid(request_id),
        "Deterministic execution identity requires exact host and Start UUIDs"
    );
    Ok(format!("Local\\TreeRail-{host_instance}-{request_id}"))
}
// Windows execution groups are installed before the managed launcher releases its gate.
#[cfg(windows)]
mod platform {
    use super::valid_identity;
    use anyhow::{Result, ensure};
    use std::{
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        ptr,
    };
    use windows_sys::Win32::System::JobObjects::*;
    pub struct ExecutionGroup {
        handle: OwnedHandle,
        name: String,
    }
    impl ExecutionGroup {
        pub fn assign(child: &dyn portable_pty::Child, identity: Option<&str>) -> Result<Self> {
            unsafe {
                let name = identity
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("Local\\TreeRail-{}", uuid::Uuid::new_v4()));
                ensure!(
                    valid_identity(&name),
                    "Invalid managed execution group identity"
                );
                let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
                let handle = CreateJobObjectW(ptr::null(), wide.as_ptr());
                ensure!(
                    !handle.is_null(),
                    "Create managed execution job: {}",
                    std::io::Error::last_os_error()
                );
                let already_exists = std::io::Error::last_os_error().raw_os_error() == Some(183);
                let group = Self {
                    handle: OwnedHandle::from_raw_handle(handle),
                    name,
                };
                ensure!(
                    !already_exists,
                    "Managed execution group already exists; reconcile its owner before starting"
                );
                let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                ensure!(
                    SetInformationJobObject(
                        handle,
                        JobObjectExtendedLimitInformation,
                        &limits as *const _ as _,
                        size_of_val(&limits) as u32
                    ) != 0,
                    "Set managed execution limits: {}",
                    std::io::Error::last_os_error()
                );
                let process = child
                    .as_raw_handle()
                    .ok_or_else(|| anyhow::anyhow!("Managed process handle unavailable"))?;
                ensure!(
                    AssignProcessToJobObject(handle, process) != 0,
                    "Assign managed execution job: {}",
                    std::io::Error::last_os_error()
                );
                Ok(group)
            }
        }
        pub fn identity(&self) -> Option<String> {
            Some(self.name.clone())
        }
        pub fn stopped(&self) -> Result<bool> {
            unsafe {
                let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
                ensure!(
                    QueryInformationJobObject(
                        self.handle.as_raw_handle(),
                        JobObjectBasicAccountingInformation,
                        &mut accounting as *mut _ as _,
                        size_of_val(&accounting) as u32,
                        ptr::null_mut()
                    ) != 0,
                    "Query managed execution: {}",
                    std::io::Error::last_os_error()
                );
                Ok(accounting.ActiveProcesses == 0)
            }
        }
        pub fn stop(&self) -> Result<()> {
            ensure!(
                unsafe { TerminateJobObject(self.handle.as_raw_handle(), 1) } != 0,
                "Stop managed execution job: {}",
                std::io::Error::last_os_error()
            );
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !self.stopped()? {
                ensure!(
                    std::time::Instant::now() < deadline,
                    "Managed descendants have not stopped; checkout remains owned"
                );
                std::thread::yield_now();
            }
            Ok(())
        }
    }
    pub fn execution_group_stopped(name: &str) -> Result<bool> {
        ensure!(
            valid_identity(name),
            "Invalid managed execution group identity"
        );
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        unsafe {
            let handle = OpenJobObjectW(0x0004, 0, wide.as_ptr());
            if handle.is_null() {
                let error = std::io::Error::last_os_error();
                if error.raw_os_error() == Some(2) {
                    return Ok(true);
                }
                return Err(error.into());
            }
            let handle = OwnedHandle::from_raw_handle(handle);
            let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
            ensure!(
                QueryInformationJobObject(
                    handle.as_raw_handle(),
                    JobObjectBasicAccountingInformation,
                    &mut accounting as *mut _ as _,
                    size_of_val(&accounting) as u32,
                    ptr::null_mut()
                ) != 0,
                "Inspect retained managed execution group: {}",
                std::io::Error::last_os_error()
            );
            Ok(accounting.ActiveProcesses == 0)
        }
    }
}
#[cfg(unix)]
mod platform {
    use anyhow::{Result, bail};
    pub struct ExecutionGroup;
    impl ExecutionGroup {
        pub fn assign(_: &dyn portable_pty::Child, _: Option<&str>) -> Result<Self> {
            bail!("Managed descendant ownership has not been validated on this platform")
        }
        pub fn identity(&self) -> Option<String> {
            None
        }
        pub fn stopped(&self) -> Result<bool> {
            bail!("Managed execution group unavailable")
        }
        pub fn stop(&self) -> Result<()> {
            bail!("Managed execution group unavailable")
        }
    }
    pub fn execution_group_stopped(_: &str) -> Result<bool> {
        bail!("Managed execution group validation is unavailable on this platform")
    }
}
pub use platform::*;
