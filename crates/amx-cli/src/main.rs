mod doctor;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use amx_core::AmxError;
use amx_core::config::Config;
use amx_index::sync::{SyncEngine, SyncReport};
use amx_store::RoConnection;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "amxcli")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Read-only readiness screen: Full Disk Access, store version, account/mailbox counts.
    Doctor,
    /// Full or incremental Tantivy index build against the configured Mail store.
    Index {
        /// Write timing + count JSON to this path (task 11's Q3 measurement).
        #[arg(long)]
        profile: Option<PathBuf>,
    },
    /// Alias for `index` without `--profile` — the steady-state incremental sync invocation.
    Sync,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Doctor => run_doctor(),
        Command::Index { profile } => run_sync(profile.as_deref()),
        Command::Sync => run_sync(None),
    }
}

fn run_doctor() -> ExitCode {
    let Some(store_path) = configured_store_path() else {
        return ExitCode::FAILURE;
    };

    match doctor::gather(&store_path) {
        Ok(report) => {
            print!("{report}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("amxcli: doctor failed: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_sync(profile_path: Option<&Path>) -> ExitCode {
    let Some(store_path) = configured_store_path() else {
        return ExitCode::FAILURE;
    };
    let config = Config::load();

    let accounts_db = match accounts_db_path(&store_path) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("amxcli: {err}");
            return ExitCode::FAILURE;
        }
    };

    let index_dir = config.cache_dir.join("index");
    let meta_path = config.cache_dir.join("meta.sqlite");
    for dir in [&index_dir, &config.cache_dir] {
        if let Err(err) = std::fs::create_dir_all(dir) {
            eprintln!("amxcli: failed to create {}: {err}", dir.display());
            return ExitCode::FAILURE;
        }
    }

    let envelope_index_path = store_path.join("MailData").join("Envelope Index");
    let started = Instant::now();
    let result = RoConnection::open(&envelope_index_path).and_then(|store_conn| {
        SyncEngine::run(
            &store_conn,
            &store_path,
            &accounts_db,
            &index_dir,
            &meta_path,
        )
    });
    let elapsed = started.elapsed();

    match result {
        Ok(report) => {
            println!("{report:?}");
            println!("elapsed: {elapsed:?}");
            if let Some(profile_path) = profile_path
                && let Err(err) = write_profile(profile_path, &report, elapsed)
            {
                eprintln!("amxcli: failed to write profile: {err}");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("amxcli: sync failed: {err}");
            ExitCode::FAILURE
        }
    }
}

fn configured_store_path() -> Option<PathBuf> {
    let config = Config::load();
    if config.store_path.is_none() {
        eprintln!(
            "amxcli: no Mail store path configured — set AMX_STORE_PATH or store_path in the config file"
        );
    }
    config.store_path
}

/// Derives `~/Library/Accounts/Accounts4.sqlite` from `store_path` (a `~/Library/Mail/V10`-shaped
/// directory) — no config field for this exists (YAGNI per the phase 2 plan), since the sibling
/// relationship between `Mail` and `Accounts` under `Library` is fixed by macOS, not configurable.
fn accounts_db_path(store_path: &Path) -> Result<PathBuf, AmxError> {
    store_path
        .parent()
        .and_then(Path::parent)
        .map(|library_dir| library_dir.join("Accounts").join("Accounts4.sqlite"))
        .ok_or_else(|| AmxError::StoreNotFound(store_path.to_path_buf()))
}

fn write_profile(
    path: &Path,
    report: &SyncReport,
    elapsed: std::time::Duration,
) -> std::io::Result<()> {
    let payload = serde_json::json!({
        "added": report.added,
        "modified": report.modified,
        "quarantined": report.quarantined,
        "reconciled_deletes": report.reconciled_deletes,
        "elapsed_ms": elapsed.as_millis(),
    });
    std::fs::write(path, serde_json::to_vec_pretty(&payload).unwrap())
}
