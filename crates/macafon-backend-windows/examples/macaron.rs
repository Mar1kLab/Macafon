//! Local hardware probe. Read-only unless explicit settings are supplied.
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use clap::Parser;
    use macafon_backend_windows::macaron;
    #[derive(Parser)]
    struct Args {
        #[arg(long, requires = "balance", value_parser = clap::value_parser!(u8).range(0..=100))]
        volume: Option<u8>,
        #[arg(long, requires = "volume", allow_hyphen_values = true, value_parser = clap::value_parser!(i8).range(-10..=10))]
        balance: Option<i8>,
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=2))]
        gain: Option<u8>,
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=4))]
        filter: Option<u8>,
        #[arg(long, value_parser = clap::value_parser!(u8).range(0..=1))]
        output_mode: Option<u8>,
    }
    let args = Args::parse();
    let devices = macaron::discover()?;
    for device in &devices {
        println!("{} -> {:?}", device.id, device.output);
    }
    let mut reports =
        macaron::setting_reports(args.gain, args.filter, args.output_mode).unwrap_or_default();
    if let (Some(volume), Some(balance)) = (args.volume, args.balance) {
        let mut pair = macaron::volume_reports(volume, balance).unwrap_or_default();
        pair.append(&mut reports);
        reports = pair;
    }
    if !reports.is_empty() {
        anyhow::ensure!(devices.len() == 1, "exactly one Macaron must be connected");
        macaron::send(&devices[0].path, &reports)?;
        println!("{} acknowledged commands", reports.len());
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
