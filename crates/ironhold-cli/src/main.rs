//! The `ironhold` command.
//!
//! - `ironhold new <name>` creates an app with sign up, log in and log out.
//! - `ironhold dev` runs the app in the current folder and rebuilds and
//!   restarts it on every change.

#![forbid(unsafe_code)]

mod dev;
mod env_file;
mod new;

use std::{path::PathBuf, process::ExitCode};

use clap::{Parser, Subcommand, ValueEnum};

pub(crate) type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;

#[derive(Parser)]
#[command(name = "ironhold", version, about = "Create and run Ironhold web apps")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new app with sign up, log in and log out ready to go.
    New {
        /// The app's name, which is also the folder it's created in.
        name: String,
        /// The database to use.
        #[arg(long, value_enum, default_value_t = Database::Sqlite)]
        db: Database,
        /// Use a local checkout of Ironhold instead of crates.io. For
        /// working on Ironhold itself.
        #[arg(long, value_name = "PATH")]
        ironhold_path: Option<PathBuf>,
    },
    /// Run the app in this folder, rebuilding and restarting it on every
    /// change.
    Dev,
}

/// Which database a new app uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub(crate) enum Database {
    /// SQLite: no database server to run. The best start for most apps.
    Sqlite,
    /// Postgres: for apps that run on several servers.
    Postgres,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::New {
            name,
            db,
            ironhold_path,
        } => new::run(&name, db, ironhold_path.as_deref()),
        Command::Dev => dev::run(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
