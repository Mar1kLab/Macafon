//! Read-only diagnostics unless explicitly invoked with `apply`.
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use clap::{Parser, Subcommand};
    use macafon_backend_windows::displays;
    use macafon_ipc::ProfileMonitor;
    #[derive(Parser)]
    struct Args {
        #[command(subcommand)]
        command: Command,
    }
    #[derive(Subcommand)]
    enum Command {
        List,
        Capture,
        Validate { file: std::path::PathBuf },
        Apply { file: std::path::PathBuf },
    }
    match Args::parse().command {
        Command::List => println!("{}", serde_json::to_string_pretty(&displays::monitors()?)?),
        Command::Capture => println!("{}", serde_json::to_string_pretty(&displays::capture()?)?),
        Command::Validate { file } => {
            let profile: Vec<ProfileMonitor> = serde_json::from_slice(&std::fs::read(file)?)?;
            let result = displays::validate(&profile)?;
            println!("Valid; missing: {:?}", result.missing);
        }
        Command::Apply { file } => {
            let profile: Vec<ProfileMonitor> = serde_json::from_slice(&std::fs::read(file)?)?;
            let result = displays::apply(&profile)?;
            println!("Applied; missing: {:?}", result.missing);
        }
    }
    Ok(())
}
#[cfg(not(windows))]
fn main() {
    eprintln!("Display profiles require Windows.");
}
