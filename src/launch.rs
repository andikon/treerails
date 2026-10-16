use crate::config::{Config, Profile};
use anyhow::{Context, Result, bail};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::{Path, PathBuf},
    process::Stdio,
};

#[derive(Debug, Clone)]
pub struct Launch {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub env: BTreeMap<String, String>,
}
fn expand(s: &str, path: &Path, branch: &str) -> String {
    s.replace("{worktree}", &path.to_string_lossy())
        .replace("{branch}", branch)
}
pub fn plan(config: &Config, profile: &Profile, path: &Path, branch: &str) -> Result<Launch> {
    let cwd = PathBuf::from(expand(&profile.cwd, path, branch));
    let cwd = if cwd.is_absolute() {
        cwd
    } else {
        path.join(cwd)
    };
    anyhow::ensure!(
        cwd.is_dir(),
        "Launch directory does not exist: {}",
        cwd.display()
    );
    let tool = expand(&profile.program, path, branch);
    let args: Vec<String> = profile
        .args
        .iter()
        .map(|s| expand(s, path, branch))
        .collect();
    let env: BTreeMap<String, String> = profile
        .env
        .iter()
        .map(|(k, v)| (k.clone(), expand(v, path, branch)))
        .collect();
    if profile.terminal != "auto" {
        let adapter = config
            .terminals
            .get(&profile.terminal)
            .with_context(|| format!("Unknown terminal adapter '{}'", profile.terminal))?;
        let mut terminal_args = Vec::new();
        for arg in &adapter.args {
            if arg == "{command}" {
                if !tool.is_empty() {
                    terminal_args.push(tool.clone().into());
                    terminal_args.extend(args.iter().map(OsString::from));
                }
            } else {
                terminal_args.push(
                    expand(arg, path, branch)
                        .replace("{cwd}", &cwd.to_string_lossy())
                        .into(),
                );
            }
        }
        return Ok(Launch {
            program: adapter.program.clone().into(),
            args: terminal_args,
            cwd,
            env,
        });
    }
    #[cfg(windows)]
    {
        let mut terminal_args: Vec<OsString> = vec![
            "-w".into(),
            "new".into(),
            "new-tab".into(),
            "--inheritEnvironment".into(),
            "--startingDirectory".into(),
            cwd.as_os_str().to_owned(),
        ];
        if !tool.is_empty() {
            terminal_args.push(tool.into());
            terminal_args.extend(args.into_iter().map(OsString::from));
        }
        Ok(Launch {
            program: "wt.exe".into(),
            args: terminal_args,
            cwd,
            env,
        })
    }
    #[cfg(target_os = "macos")]
    {
        // Terminal.app accepts a command via AppleScript. Quote each shell argument and then
        // encode the complete command as an AppleScript string, never interpolate raw input.
        let shell_quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
        let mut script = format!("cd -- {}", shell_quote(&cwd.to_string_lossy()));
        if !tool.is_empty() || !env.is_empty() {
            script.push_str(" && env");
            for (key, value) in &env {
                script.push(' ');
                script.push_str(&shell_quote(&format!("{key}={value}")));
            }
            script.push(' ');
            script.push_str(&shell_quote(if tool.is_empty() {
                "/bin/zsh"
            } else {
                &tool
            }));
            for arg in args {
                script.push(' ');
                script.push_str(&shell_quote(&arg));
            }
            if tool.is_empty() {
                script.push_str(" -l");
            }
        }
        let encoded = serde_json::to_string(&script)?;
        Ok(Launch {
            program: "osascript".into(),
            args: vec![
                "-e".into(),
                format!("tell application \"Terminal\" to do script {encoded}").into(),
                "-e".into(),
                "tell application \"Terminal\" to activate".into(),
            ],
            cwd,
            env,
        })
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let terminal = std::env::var("TERMINAL")
            .ok()
            .or_else(|| {
                ["x-terminal-emulator", "gnome-terminal", "konsole", "xterm"]
                    .iter()
                    .find(|p| available(p))
                    .map(|s| s.to_string())
            })
            .context("No terminal found; configure a terminal adapter")?;
        let name = Path::new(&terminal)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        let mut terminal_args: Vec<OsString> = vec![];
        if name.contains("gnome-terminal") {
            terminal_args.extend([
                "--window".into(),
                "--working-directory".into(),
                cwd.as_os_str().to_owned(),
            ]);
            if !tool.is_empty() {
                terminal_args.push("--".into());
            }
        } else if name.contains("konsole") {
            terminal_args.extend([
                "--separate".into(),
                "--workdir".into(),
                cwd.as_os_str().to_owned(),
            ]);
            if !tool.is_empty() {
                terminal_args.push("-e".into());
            }
        } else if !tool.is_empty() {
            terminal_args.push("-e".into());
        }
        if !tool.is_empty() {
            terminal_args.push(tool.into());
            terminal_args.extend(args.into_iter().map(OsString::from));
        }
        Ok(Launch {
            program: terminal.into(),
            args: terminal_args,
            cwd,
            env,
        })
    }
}
fn available(program: &str) -> bool {
    let path = Path::new(program);
    if path.is_file() {
        return true;
    }
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|p| {
            if p.join(program).is_file() {
                return true;
            }
            cfg!(windows)
                && ["exe", "cmd", "bat"]
                    .iter()
                    .any(|ext| p.join(format!("{program}.{ext}")).is_file())
        })
    })
}
pub fn launch(config: &Config, name: &str, path: &Path, branch: &str) -> Result<()> {
    let profile = config
        .profiles
        .get(name)
        .with_context(|| format!("Unknown launch profile: {name}"))?;
    if !profile.program.is_empty() && !available(&expand(&profile.program, path, branch)) {
        bail!(
            "'{}' is unavailable; choose the terminal profile or adjust your configuration",
            profile.program
        );
    }
    let launch = plan(config, profile, path, branch)?;
    let mut cmd = std::process::Command::new(&launch.program);
    cmd.args(&launch.args)
        .current_dir(&launch.cwd)
        .envs(&launch.env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Cannot launch {}; configure a terminal adapter",
            launch.program.to_string_lossy()
        )
    })?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
