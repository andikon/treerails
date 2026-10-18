//! Private executable roles, shared by standalone downloads and the companion host.
use anyhow::{Context, Result, ensure};
use std::ffi::OsString;

/// Dispatch before interactive CLI startup. Ordinary discovery/listing never enters this path.
pub async fn dispatch() -> Result<bool> {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref() {
        Some(role) if role == "--session-host" => {
            let endpoint = args
                .next()
                .context("A private session host endpoint is required")?;
            ensure!(args.next().is_none(), "Unexpected session host argument");
            super::runtime::serve(endpoint.to_str().context("Host endpoint must be Unicode")?)
                .await?;
            Ok(true)
        }
        Some(role) if role == "--managed-child" => {
            managed_child(args)?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

pub fn managed_child(mut args: impl Iterator<Item = OsString>) -> Result<()> {
    // No agent starts until the host installs descendant ownership and releases this gate.
    let mut line = String::new();
    ensure!(
        std::io::stdin().read_line(&mut line)? > 0,
        "Managed launch gate closed"
    );
    ensure!(
        line.trim() == "treerail-start",
        "Invalid managed launch gate"
    );
    let program = args.next().context("Managed child program is required")?;
    let status = std::process::Command::new(program).args(args).status()?;
    std::process::exit(status.code().unwrap_or(1));
}
