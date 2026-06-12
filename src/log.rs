use std::fs::OpenOptions;
use std::path::PathBuf;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

struct LogFile {
    dir: PathBuf,
}

// SAFETY: File is Send + Sync. The make_writer opens a new file handle
// each time so there is no shared state between writers.
unsafe impl Sync for LogFile {}

impl<'a> MakeWriter<'a> for LogFile {
    type Writer = std::fs::File;

    fn make_writer(&'a self) -> Self::Writer {
        let path = self.dir.join("opencode.log");
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap_or_else(|_| {
                let _ = std::fs::create_dir_all(&self.dir);
                std::fs::File::create(&path).unwrap()
            })
    }
}

pub fn init(level: &str) {
    let log_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("opencode-rs")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    let log_file = LogFile { dir: log_dir };

    let filter = format!("opencode_rs={}", level);

    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(log_file)
        .with_ansi(false)
        .with_filter(tracing_subscriber::filter::EnvFilter::new(&filter));

    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_filter(tracing_subscriber::filter::LevelFilter::WARN);

    tracing_subscriber::registry()
        .with(stderr_layer)
        .with(file_layer)
        .init();
}
