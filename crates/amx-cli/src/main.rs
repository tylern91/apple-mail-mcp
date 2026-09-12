mod doctor;

use std::process::ExitCode;

use amx_core::config::Config;

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        Some("doctor") => run_doctor(),
        Some(other) => {
            eprintln!("amxcli: unknown command {other:?}\nusage: amxcli doctor");
            ExitCode::FAILURE
        }
        None => {
            eprintln!("usage: amxcli doctor");
            ExitCode::FAILURE
        }
    }
}

fn run_doctor() -> ExitCode {
    let config = Config::load();
    let Some(store_path) = config.store_path else {
        eprintln!(
            "amxcli: no Mail store path configured — set AMX_STORE_PATH or store_path in the config file"
        );
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
