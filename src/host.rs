//! Provider-independent invariants for the persistent session host.
//!
//! This module does not infer agent attention or implement a terminal provider.
//! A checkout lease must remain held until managed execution is verifiably stopped.
use anyhow::{Context, Result, bail, ensure};
use fs2::FileExt;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

/// Reject incompatible peers before accepting any state-changing commands.
pub fn handshake(peer_version: u32) -> Result<()> {
    ensure!(
        peer_version == PROTOCOL_VERSION,
        "Session host protocol {peer_version} is incompatible with client protocol {PROTOCOL_VERSION}; detach or explicitly stop the existing host before upgrading"
    );
    Ok(())
}

/// A bounded length-prefixed JSON message; oversized payloads never reach the transport.
pub fn write_frame<W: Write, T: Serialize>(writer: &mut W, value: &T) -> Result<()> {
    let payload = serde_json::to_vec(value)?;
    ensure!(
        payload.len() <= MAX_FRAME_BYTES,
        "Session host message exceeds frame limit"
    );
    writer.write_all(&(payload.len() as u32).to_be_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

/// Validate length before allocating; truncated messages are errors, never commands.
pub fn read_frame<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<T> {
    let mut header = [0; 4];
    reader
        .read_exact(&mut header)
        .context("Read session host frame header")?;
    let length = u32::from_be_bytes(header) as usize;
    ensure!(
        length > 0 && length <= MAX_FRAME_BYTES,
        "Invalid session host frame length {length}"
    );
    let mut payload = vec![0; length];
    reader
        .read_exact(&mut payload)
        .context("Read complete session host frame")?;
    serde_json::from_slice(&payload).context("Decode session host frame")
}

/// OS-backed exclusion across hosts, tickets and configuration files.
///
/// Git resolves each physical checkout's own metadata directory, so aliases and
/// nested paths share a lease while linked worktrees can execute independently.
/// The empty lock file is retained: unlinking a held lock would permit split ownership.
#[derive(Debug)]
pub struct CheckoutLease {
    _file: File,
    metadata: PathBuf,
}
impl CheckoutLease {
    pub async fn acquire(checkout: &Path) -> Result<Self> {
        let output = crate::git::run(checkout, ["rev-parse", "--absolute-git-dir"]).await?;
        let metadata = std::fs::canonicalize(String::from_utf8(output)?.trim())
            .context("Resolve checkout metadata")?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(metadata.join("treerail-session.lock"))
            .context("Open checkout ownership lease")?;
        file.try_lock_exclusive()
            .context("Checkout is owned by another managed agent session")?;
        Ok(Self {
            _file: file,
            metadata,
        })
    }
    pub fn metadata_directory(&self) -> &Path {
        &self.metadata
    }
}

/// Opaque input authority, scoped to one live host and invalidated on takeover.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlToken(u64);

#[derive(Debug, Default)]
pub struct InputControl {
    generation: u64,
    owner: Option<(String, ControlToken)>,
}
impl InputControl {
    /// Attaching a viewer never silently takes another client's keyboard.
    pub fn attach(&mut self, client: &str, takeover: bool) -> Result<ControlToken> {
        ensure!(!client.is_empty(), "Client identity cannot be empty");
        if let Some((owner, token)) = &self.owner {
            if owner == client {
                return Ok(*token);
            }
            ensure!(
                takeover,
                "Another UI controls this terminal; explicit takeover required"
            );
        }
        self.generation = self
            .generation
            .checked_add(1)
            .context("Input control generation exhausted")?;
        let token = ControlToken(self.generation);
        self.owner = Some((client.to_owned(), token));
        Ok(token)
    }
    pub fn authorize(&self, client: &str, token: ControlToken) -> Result<()> {
        if self
            .owner
            .as_ref()
            .is_some_and(|(owner, active)| owner == client && *active == token)
        {
            return Ok(());
        }
        bail!("UI no longer controls this terminal; attach explicitly before sending input")
    }
    /// Releasing the keyboard does not release a checkout or stop an agent.
    pub fn detach(&mut self, client: &str, token: ControlToken) -> Result<()> {
        self.authorize(client, token)?;
        self.owner = None;
        Ok(())
    }
}

mod execution;
pub mod ipc;
pub mod runtime;
