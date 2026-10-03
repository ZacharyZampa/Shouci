use std::process::ExitCode;

fn main() -> ExitCode {
    match shouci_tui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("shouci-tui: {err}");
            ExitCode::FAILURE
        }
    }
}
