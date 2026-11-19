//! Точка входа: менюбар macOS + фоновый наблюдатель, а окно настроек
//! запускается отдельным процессом (`--settings`), чтобы не держать память.

use desktop_cleaner::engine::start_watcher;
use desktop_cleaner::ui;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

fn main() {
    if let Err(err) = run() {
        eprintln!("desktop_cleaner: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = dirs::config_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "нет папки настроек"))?
        .join("DesktopCleaner");
    fs::create_dir_all(&data_dir)?;

    let config_path = data_dir.join("config.json");
    let journal_path = data_dir.join("journal.json");
    let desktop_dir = dirs::desktop_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "нет домашней папки"))?;

    // Дочерний процесс: только окно настроек, без менюбара.
    if std::env::args().skip(1).any(|arg| arg == "--settings") {
        ui::run_settings_window(config_path, journal_path, desktop_dir)?;
        return Ok(());
    }

    // Первый запуск: сразу показываем настройки.
    if !config_path.exists() {
        if let Err(err) = ui::spawn_settings_window() {
            eprintln!("desktop_cleaner: не удалось открыть настройки: {err}");
        }
    }

    let stop_signal = Arc::new(AtomicBool::new(false));
    spawn_watcher_when_ready(
        config_path.clone(),
        journal_path.clone(),
        desktop_dir.clone(),
        stop_signal.clone(),
    );

    ui::run_tray(config_path, journal_path, desktop_dir, stop_signal.clone())?;
    stop_signal.store(true, Ordering::Relaxed);

    Ok(())
}

/// Не запускает уборку, пока пользователь не сохранил config.json,
/// чтобы дефолтные настройки не тронули рабочий стол на первом запуске.
fn spawn_watcher_when_ready(
    config_path: PathBuf,
    journal_path: PathBuf,
    desktop_dir: PathBuf,
    stop_signal: Arc<AtomicBool>,
) {
    thread::spawn(move || {
        while !config_path.exists() {
            if stop_signal.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(200));
        }

        start_watcher(desktop_dir, config_path, journal_path, stop_signal);
    });
}
