use std::path::PathBuf;

use directories::ProjectDirs;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::EnvFilter;

/// Initialize file-based logging. Stdout belongs to the TUI, so all logs go to
/// a rolling file under the platform data dir. The returned guard must be kept
/// alive for the lifetime of the program or buffered logs are dropped.
pub fn init() -> WorkerGuard {
    let dir = log_dir();
    let _ = std::fs::create_dir_all(&dir);

    let file_appender = tracing_appender::rolling::daily(&dir, "mint.log");
    let (writer, guard) = tracing_appender::non_blocking(file_appender);

    let filter = EnvFilter::try_from_env("MINT_LOG").unwrap_or_else(|_| EnvFilter::new("info"));

    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true)
        .try_init();

    guard
}

fn log_dir() -> PathBuf {
    ProjectDirs::from("dev", "mint", "mint-cli")
        .map(|d| d.data_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from(".mint/logs"))
}
