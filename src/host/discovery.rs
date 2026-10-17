use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub(super) fn runtime_file(endpoint: &str, suffix: &str) -> Result<PathBuf> {
    ensure!(
        !endpoint.is_empty()
            && endpoint.len() <= 100
            && endpoint
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-'),
        "Invalid host endpoint"
    );
    #[cfg(windows)]
    let directory = directories::BaseDirs::new()
        .context("Current user data directory unavailable")?
        .data_local_dir()
        .join("treerails")
        .join("runtime");
    #[cfg(unix)]
    let directory = PathBuf::from(
        std::env::var_os("XDG_RUNTIME_DIR").context("Private runtime directory unavailable")?,
    )
    .join("treerails");
    std::fs::create_dir_all(&directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(directory.join(format!("{endpoint}.{suffix}")))
}
fn open_lock(endpoint: &str, suffix: &str) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(runtime_file(endpoint, suffix)?)
        .context("Open host lifecycle lock")
}
pub(super) fn host_lock(endpoint: &str) -> Result<File> {
    let file = open_lock(endpoint, "host-lock")?;
    file.try_lock_exclusive()
        .context("A session host already owns this endpoint")?;
    Ok(file)
}
pub(super) fn startup_lock(endpoint: &str) -> Result<File> {
    let file = open_lock(endpoint, "startup-lock")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => return Ok(file),
            Err(error)
                if (error.kind() == std::io::ErrorKind::WouldBlock
                    || error.raw_os_error() == fs2::lock_contended_error().raw_os_error())
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => return Err(error).context("Coordinate session host startup"),
        }
    }
}
pub fn endpoint_for_config(config: &Path) -> Result<String> {
    let absolute = if config.is_absolute() {
        config.to_owned()
    } else {
        std::env::current_dir()?.join(config)
    };
    let resolved = if absolute.exists() {
        std::fs::canonicalize(&absolute)?
    } else {
        let parent = absolute.parent().context("Configuration has no parent")?;
        std::fs::canonicalize(parent)?.join(
            absolute
                .file_name()
                .context("Configuration has no filename")?,
        )
    };
    let scope = directories::BaseDirs::new()
        .context("Current user directory unavailable")?
        .data_local_dir()
        .to_owned();
    let identity = format!("{}\n{}", scope.display(), resolved.display());
    #[cfg(windows)]
    let identity = identity.to_lowercase();
    // Fixed, version-independent FNV identity: versions meet at the same host and
    // negotiate rather than launching competing hosts after an upgrade.
    let hash = identity.bytes().fold(0xcbf29ce484222325u64, |value, byte| {
        (value ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    });
    Ok(format!("host-{hash:016x}"))
}
