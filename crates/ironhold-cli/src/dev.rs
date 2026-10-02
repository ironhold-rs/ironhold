//! `ironhold dev`: build, run, and rebuild on every change.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Child, Command},
    sync::mpsc::{Receiver, RecvTimeoutError},
    time::Duration,
};

use notify_debouncer_mini::{DebounceEventResult, new_debouncer, notify::RecursiveMode};

use crate::{Result, env_file};

/// Folders and files whose changes trigger a rebuild.
const WATCHED: &[&str] = &[
    "src",
    "migrations",
    "templates",
    "static",
    "Cargo.toml",
    ".env",
];

pub(crate) fn run() -> Result<()> {
    let project = Project::load()?;
    let (sender, changes) = std::sync::mpsc::channel();
    let mut debouncer = new_debouncer(Duration::from_millis(200), sender)?;
    for path in WATCHED.iter().map(Path::new).filter(|p| p.exists()) {
        debouncer.watcher().watch(path, RecursiveMode::Recursive)?;
    }

    let mut server: Option<Child> = None;
    loop {
        say("building...");
        if build(&project)? {
            if let Some(mut old) = server.take() {
                stop(&mut old);
            }
            server = Some(start(&project)?);
        } else if server.is_some() {
            say("build failed; the previous version is still running. Fix the error and save.");
        } else {
            say("build failed. Fix the error and save.");
        }
        wait_for_change(&changes, &mut server)?;
    }
}

/// The binary to build and run.
struct Project {
    bin: String,
    executable: PathBuf,
}

impl Project {
    fn load() -> Result<Self> {
        let output = Command::new(cargo())
            .args(["metadata", "--no-deps", "--format-version", "1"])
            .output()?;
        if !output.status.success() {
            return Err("no Cargo project here: run `ironhold dev` in your app's folder".into());
        }
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        let manifest = env::current_dir()?.join("Cargo.toml").canonicalize()?;

        let packages = metadata["packages"].as_array().cloned().unwrap_or_default();
        let package = packages
            .iter()
            .find(|p| {
                p["manifest_path"]
                    .as_str()
                    .and_then(|m| Path::new(m).canonicalize().ok())
                    .is_some_and(|m| m == manifest)
            })
            .or_else(|| packages.first())
            .ok_or("no package found in Cargo.toml")?;
        let bin = package["targets"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|t| {
                t["kind"]
                    .as_array()
                    .is_some_and(|k| k.iter().any(|k| k == "bin"))
            })
            .and_then(|t| t["name"].as_str())
            .ok_or("this package has no binary to run")?
            .to_owned();
        let target_dir = metadata["target_directory"]
            .as_str()
            .ok_or("cargo metadata has no target directory")?;
        let executable = Path::new(target_dir)
            .join("debug")
            .join(format!("{bin}{}", env::consts::EXE_SUFFIX));
        Ok(Self { bin, executable })
    }
}

fn build(project: &Project) -> Result<bool> {
    let status = Command::new(cargo())
        .args(["build", "--bin", &project.bin])
        .status()?;
    Ok(status.success())
}

fn start(project: &Project) -> Result<Child> {
    let mut command = Command::new(&project.executable);
    // `.env` fills in variables that aren't already set in the shell.
    if let Ok(text) = fs::read_to_string(".env") {
        for (key, value) in env_file::parse(&text) {
            if env::var_os(&key).is_none() {
                command.env(key, value);
            }
        }
    }
    say("starting the app");
    Ok(command.spawn()?)
}

fn stop(server: &mut Child) {
    let _ = server.kill();
    let _ = server.wait();
}

/// Blocks until a watched file changes. Reports if the app exits on its
/// own in the meantime (for example a crash at startup).
fn wait_for_change(
    changes: &Receiver<DebounceEventResult>,
    server: &mut Option<Child>,
) -> Result<()> {
    loop {
        match changes.recv_timeout(Duration::from_millis(500)) {
            Ok(Ok(events)) if !events.is_empty() => {
                // Coalesce a burst of saves into one rebuild.
                while changes.try_recv().is_ok() {}
                return Ok(());
            }
            Ok(Ok(_)) => {}
            Ok(Err(error)) => say(&format!("file watching error: {error}")),
            Err(RecvTimeoutError::Timeout) => {
                if let Some(child) = server
                    && let Ok(Some(status)) = child.try_wait()
                {
                    say(&format!(
                        "the app stopped ({status}). Waiting for changes..."
                    ));
                    *server = None;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err("file watcher stopped".into());
            }
        }
    }
}

fn cargo() -> String {
    env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

fn say(message: &str) {
    eprintln!("\x1b[1;36mironhold\x1b[0m {message}");
}
