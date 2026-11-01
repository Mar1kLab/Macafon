//! Compiled by the downloader tests as a standalone, network-free fake executable.
use std::{fs, io::Write, path::PathBuf, process::Command, thread, time::Duration};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--fake-child") {
        let path = PathBuf::from(&args[1]);
        loop {
            let mut file = fs::OpenOptions::new().create(true).append(true).open(&path).unwrap();
            file.write_all(b"heartbeat\n").unwrap();
            thread::sleep(Duration::from_millis(10));
        }
    }
    if args.iter().any(|arg| arg == "--version") {
        println!("2026.11.01");
        return;
    }
    let output = PathBuf::from(args[args.iter().position(|arg| arg == "-o").unwrap() + 1].replace("%(ext)s", "wav").replace("%%", "%"));
    let url = args.last().unwrap();
    if url.ends_with("/fail") {
        eprintln!("warning: a previous diagnostic");
        eprintln!("ERROR: Requested format is not available");
        std::process::exit(1);
    }
    if url.contains("/slow") {
        fs::write(output.with_extension("wav.part"), b"partial").unwrap();
        let heartbeat = output.parent().unwrap().parent().unwrap().parent().unwrap().join("heartbeat");
        let mut child = Command::new(std::env::current_exe().unwrap()).arg("--fake-child").arg(heartbeat).spawn().unwrap();
        println!("MACAFON_PROGRESS: 10.0%");
        std::io::stdout().flush().unwrap();
        thread::sleep(Duration::from_secs(60));
        let _ = child.kill();
        let _ = child.wait();
        return;
    }
    println!("MACAFON_TITLE:\"A: bell / sound.\"");
    for pct in [12.5, 60.0, 100.0] {
        println!("MACAFON_PROGRESS: {pct}%");
        std::io::stdout().flush().unwrap();
        thread::sleep(Duration::from_millis(120));
    }
    println!("MACAFON_CONVERTING");
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&132u32.to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&48000u32.to_le_bytes());
    wav.extend_from_slice(&96000u32.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&96u32.to_le_bytes());
    wav.extend_from_slice(&[0; 96]);
    fs::write(&output, wav).unwrap();
    // JSON-quote the Windows path without needing serde in this standalone fixture.
    println!("MACAFON_FILE:\"{}\"", output.to_str().unwrap().replace('\\', "\\\\").replace('"', "\\\""));
}
