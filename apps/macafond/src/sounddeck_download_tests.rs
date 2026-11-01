use super::*;
use crate::sounddeck::SoundDeck;
use macafon_config::SoundDeckConfig;

#[test]
fn url_validation_and_arguments_keep_untrusted_input_after_separator() {
    for url in [
        "https://example.com/watch?v=a&x=--exec",
        "http://localhost:8000/a",
        "HTTPS://example.com/",
    ] {
        validate_url(url).expect("http URL");
        for ffmpeg in [false, true] {
            let args = arguments(url, Path::new("stage/media.%(ext)s"), ffmpeg);
            let args: Vec<_> = args.iter().map(|arg| arg.to_string_lossy()).collect();
            assert_eq!(&args[args.len() - 2..], ["--", url]);
            for pair in [
                ["--max-filesize", "200M"],
                ["--match-filter", "duration <= 600"],
                ["--max-downloads", "1"],
            ] {
                assert!(args.windows(2).any(|args| args == pair));
            }
            for flag in [
                "--ignore-config",
                "--no-playlist",
                "--no-exec",
                "--newline",
                "--restrict-filenames",
            ] {
                assert!(args.iter().any(|arg| arg == flag));
            }
            assert_eq!(args.iter().any(|arg| arg == "-x"), ffmpeg);
            if !ffmpeg {
                let selection = &args[args
                    .iter()
                    .position(|arg| arg == "-f")
                    .expect("format flag")
                    + 1];
                for ext in library::SUPPORTED_EXTENSIONS {
                    assert!(selection.contains(&format!("[ext={ext}]")));
                }
                assert!(!selection.contains("/best") || !selection.ends_with("/best"));
            }
        }
    }
    for url in [
        "--exec calc.exe",
        "file:///tmp/a",
        "ftp://example.com/a",
        "https://",
        "https://a.test/\n--exec",
        "https://a.test/ a",
        "http:invalid",
        "",
    ] {
        assert_eq!(
            validate_url(url).expect_err("reject URL").code,
            ErrorCode::InvalidParams,
            "{url:?}"
        );
    }
}

#[test]
fn machine_progress_metadata_errors_and_windows_names() {
    assert_eq!(
        parse_line("MACAFON_PROGRESS: 42.5%"),
        Some(Line::Progress(42.5))
    );
    assert_eq!(
        parse_line("MACAFON_PROGRESS: 120%"),
        Some(Line::Progress(100.0))
    );
    for line in [
        "MACAFON_PROGRESS: NA",
        "MACAFON_PROGRESS: NaN%",
        "MACAFON_PROGRESS: inf%",
        "[download] 40%",
    ] {
        assert_eq!(parse_line(line), None);
    }
    assert_eq!(
        parse_line("MACAFON_TITLE:\"A title\\nwith a newline\""),
        Some(Line::Title("A title\nwith a newline".into()))
    );
    assert_eq!(parse_line("MACAFON_CONVERTING"), Some(Line::Converting));
    assert_eq!(
        short_error("ERROR: [site] Sound is private\n", true),
        "[site] Sound is private"
    );
    assert!(
        short_error("ERROR: Requested format is not available", false).contains("install ffmpeg")
    );
    assert!(short_error(&"x".repeat(10000), true).len() <= 300);
    assert!(!short_error("\u{1b}\0", true).chars().any(char::is_control));
    for title in [
        "CON",
        "..",
        " LPT1. ",
        "A: bell / sound.",
        "Unplayable",
        "COM¹",
        "",
        ".hidden",
    ] {
        library::validate_name(&title_name(title)).expect("Windows-safe media title");
    }
}

#[test]
fn progress_snapshots_are_aggregated_and_throttled_across_jobs() {
    let mut downloads = Downloads::default();
    let mut senders = Vec::new();
    for index in 0..3 {
        let (send, updates) = mpsc::sync_channel(8);
        senders.push(send);
        downloads.jobs.push(Job {
            status: SoundDeckDownload {
                id: index.to_string(),
                url: "https://example.test/a".into(),
                title: None,
                category: None,
                state: State::Downloading,
                progress_pct: None,
                error: None,
                sound_id: None,
            },
            cancel: Arc::default(),
            progress: Arc::new(AtomicU32::new(10.0f32.to_bits())),
            updates,
            thread: None,
        });
    }
    let now = Instant::now();
    downloads.poll(now);
    assert_eq!(
        downloads.take_events().len(),
        1,
        "one snapshot for all three jobs"
    );
    for job in &downloads.jobs {
        job.progress.store(20.0f32.to_bits(), Ordering::Relaxed);
    }
    downloads.poll(now + Duration::from_millis(249));
    assert!(downloads.take_events().is_empty());
    downloads.poll(now + Duration::from_millis(250));
    let events = downloads.take_events();
    assert_eq!(events.len(), 1);
    assert!(events[0].iter().all(|job| job.progress_pct == Some(20.0)));
}

struct Fixture {
    root: PathBuf,
    executable: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "macafon-download-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir(&root).expect("test root");
        let source = root.join("fake_ytdlp.rs");
        std::fs::write(&source, include_str!("../tests/fixtures/fake_ytdlp.rs"))
            .expect("fixture source");
        let executable = root.join(if cfg!(windows) {
            "fake-ytdlp.exe"
        } else {
            "fake-ytdlp"
        });
        let status = Command::new("rustc")
            .arg("--edition=2024")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .expect("compile fake tool");
        assert!(status.success());
        Self { root, executable }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).expect("fixture cleanup");
    }
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "worker timed out"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn fake_tool_download_finalizes_and_cancel_kills_tree_and_cleans_staging() {
    let fixture = Fixture::new();
    let config = SoundDeckConfig {
        folder: Some(fixture.root.join("library 100%")),
        downloader_path: Some(fixture.executable.clone()),
        ..Default::default()
    };
    let mut deck = SoundDeck::new(&config, &fixture.root.join("state"));
    wait_until(|| {
        deck.tick(&config, Instant::now());
        deck.downloads.detection.is_none()
    });
    assert!(deck.downloader().available);
    assert_eq!(deck.downloader().version.as_deref(), Some("2026.11.01"));
    // Deterministically exercise the no-ffmpeg selection regardless of the developer's PATH.
    deck.downloads.info.ffmpeg = false;
    let id = deck
        .download("https://example.test/sound".into(), Some("Memes".into()))
        .expect("start");
    let mut states = Vec::new();
    wait_until(|| {
        deck.tick(&config, Instant::now());
        for event in deck.download_events() {
            states.push(event.iter().find(|job| job.id == id).expect("job").state);
        }
        deck.downloads()
            .iter()
            .any(|job| job.id == id && job.state == State::Done)
    });
    for state in [
        State::Queued,
        State::Downloading,
        State::Converting,
        State::Done,
    ] {
        assert!(states.contains(&state), "missing {state:?}: {states:?}");
    }
    let sound_id = deck.downloads()[0]
        .sound_id
        .clone()
        .expect("completed sound id");
    assert_eq!(sound_id, "Memes/A_ bell _ sound.wav");
    assert!(config.folder().join(&sound_id).is_file());
    assert!(deck.entries().iter().any(|entry| entry.id == sound_id));
    let second = deck
        .download("https://example.test/sound".into(), Some("Memes".into()))
        .expect("second import");
    wait_until(|| {
        deck.tick(&config, Instant::now());
        deck.downloads()
            .iter()
            .any(|job| job.id == second && job.state == State::Done)
    });
    assert!(
        config
            .folder()
            .join("Memes/A_ bell _ sound (2).wav")
            .is_file()
    );
    deck.download_dismiss(&id).expect("dismiss done");
    assert!(!deck.downloads().iter().any(|job| job.id == id));
    let failed = deck
        .download("https://example.test/fail".into(), None)
        .expect("start failing job");
    wait_until(|| {
        deck.tick(&config, Instant::now());
        deck.downloads()
            .iter()
            .any(|job| job.id == failed && job.state == State::Failed)
    });
    assert!(
        deck.downloads()
            .iter()
            .find(|job| job.id == failed)
            .expect("failed job")
            .error
            .as_deref()
            .expect("error")
            .contains("install ffmpeg")
    );
    for suffix in ["slow1", "slow2", "slow3"] {
        deck.download(format!("https://example.test/{suffix}"), None)
            .expect("three jobs allowed");
    }
    assert_eq!(
        deck.download("https://example.test/fourth".into(), None)
            .expect_err("limit")
            .code,
        ErrorCode::Busy
    );
    let heartbeat = config.folder().join("heartbeat");
    wait_until(|| {
        heartbeat
            .metadata()
            .is_ok_and(|metadata| metadata.len() > 0)
    });
    let slow: Vec<_> = deck
        .downloads()
        .into_iter()
        .filter(|job| job.url.contains("/slow"))
        .map(|job| job.id)
        .collect();
    let stage = config.folder().join(".macafon-uploads");
    for id in &slow {
        deck.download_cancel(id).expect("cancel");
        deck.download_dismiss(id)
            .expect("dismiss cancelled without waiting");
    }
    wait_until(|| {
        deck.tick(&config, Instant::now());
        slow.iter().all(|id| !stage.join(id).exists())
    });
    let length = heartbeat.metadata().expect("heartbeat").len();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(
        heartbeat.metadata().expect("heartbeat").len(),
        length,
        "ffmpeg-like child must be dead"
    );
    assert!(std::fs::read_dir(&stage).expect("staging").next().is_none());
    // Library switches and daemon shutdown must cancel jobs without finalizing in another root.
    let switched = deck
        .download("https://example.test/slow-switch".into(), None)
        .expect("switch job");
    wait_until(|| stage.join(&switched).exists());
    deck.set_folder(fixture.root.join("other-library"));
    wait_until(|| {
        deck.tick(&config, Instant::now());
        !stage.join(&switched).exists()
    });
    assert_eq!(
        deck.downloads()
            .iter()
            .find(|job| job.id == switched)
            .expect("switched job")
            .state,
        State::Cancelled
    );
    let shutdown = deck
        .download("https://example.test/slow-shutdown".into(), None)
        .expect("shutdown job");
    let shutdown_stage = fixture
        .root
        .join("other-library/.macafon-uploads")
        .join(&shutdown);
    wait_until(|| shutdown_stage.exists());
    drop(deck);
    assert!(!shutdown_stage.exists());
}
