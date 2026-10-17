//! Windows execution groups are installed before the managed launcher releases its gate.
#[cfg(windows)]
mod platform {
    use anyhow::{Result, ensure};
    use std::{
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        ptr,
    };
    use windows_sys::Win32::System::JobObjects::*;
    pub struct ExecutionGroup(OwnedHandle);
    impl ExecutionGroup {
        pub fn assign(child: &dyn portable_pty::Child) -> Result<Self> {
            unsafe {
                let handle = CreateJobObjectW(ptr::null(), ptr::null());
                ensure!(
                    !handle.is_null(),
                    "Create managed execution job: {}",
                    std::io::Error::last_os_error()
                );
                let group = Self(OwnedHandle::from_raw_handle(handle));
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
        pub fn stopped(&self) -> Result<bool> {
            unsafe {
                let mut accounting: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
                ensure!(
                    QueryInformationJobObject(
                        self.0.as_raw_handle(),
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
                unsafe { TerminateJobObject(self.0.as_raw_handle(), 1) } != 0,
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
}
#[cfg(unix)]
mod platform {
    use anyhow::{Result, bail};
    pub struct ExecutionGroup;
    impl ExecutionGroup {
        pub fn assign(_: &dyn portable_pty::Child) -> Result<Self> {
            bail!("Managed descendant ownership has not been validated on this platform")
        }
        pub fn stopped(&self) -> Result<bool> {
            bail!("Managed execution group unavailable")
        }
        pub fn stop(&self) -> Result<()> {
            bail!("Managed execution group unavailable")
        }
    }
}
pub use platform::*;
