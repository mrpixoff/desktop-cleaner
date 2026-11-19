//! Движок очистки: разовый проход и фоновое наблюдение за рабочим столом.

use crate::archive::{cleanup_expired_archive, move_to_archive};
use crate::classify::should_archive;
use crate::config::{Config, Timer};
use crate::journal::{Journal, MovedItem};
use chrono::Local;
use notify::{RecursiveMode, Watcher};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// Пауза тишины после последнего события в режиме `Immediate`.
const DEBOUNCE: Duration = Duration::from_secs(3);
/// Как часто проверять возраст файлов в режиме `Minutes`.
const MINUTES_POLL_INTERVAL: Duration = Duration::from_secs(30);
/// Как часто сверять время в режиме `Daily`.
const DAILY_POLL_INTERVAL: Duration = Duration::from_secs(5);
/// Шаг ожидания событий, чтобы замечать сигнал остановки.
const POLL_TICK: Duration = Duration::from_millis(200);
/// Как часто перечитывать config.json, чтобы замечать паузу/возобновление.
const CONFIG_REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// Как часто проверять срок хранения архива, если автоудаление включено.
const ARCHIVE_CLEANUP_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Один проход уборки верхнего уровня `desktop_dir`.
///
/// Внутрь папок не заходим: папка либо переносится целиком, либо остаётся.
/// Перемещения записываются одной пачкой в журнал, если было перемещено
/// хотя бы что-то. Возвращает количество убранных объектов.
pub fn run_cleanup_once(
    desktop_dir: &Path,
    config: &Config,
    journal_path: &Path,
) -> io::Result<usize> {
    run_cleanup_filtered(desktop_dir, config, journal_path, None)
}

/// Фоновый цикл очистки. Рассчитан на запуск в отдельном потоке.
///
/// * при старте делает проход, если [`Timer::Immediate`];
/// * слушает изменения `desktop_dir` через notify (FSEvents на macOS);
/// * перечитывает `config_path` при каждом событии и перед каждой уборкой;
/// * поддерживает все три режима таймера;
/// * завершается, когда `stop_signal` выставлен в `true`.
pub fn start_watcher(
    desktop_dir: PathBuf,
    config_path: PathBuf,
    journal_path: PathBuf,
    stop_signal: Arc<AtomicBool>,
) {
    let (sender, receiver) = mpsc::channel();
    let mut watcher = match notify::recommended_watcher(
        move |result: notify::Result<notify::Event>| {
            let _ = sender.send(result);
        },
    ) {
        Ok(watcher) => watcher,
        Err(err) => {
            eprintln!("desktop_cleaner: не удалось создать наблюдателя: {err}");
            return;
        }
    };

    if let Err(err) = watcher.watch(&desktop_dir, RecursiveMode::NonRecursive) {
        eprintln!(
            "desktop_cleaner: не удалось наблюдать за {}: {err}",
            desktop_dir.display()
        );
        return;
    }

    let mut config = Config::load_from(&config_path).unwrap_or_default();

    let mut last_archive_cleanup = Instant::now();

    if config.enabled && matches!(&config.timer, Timer::Immediate) {
        run_and_log(&desktop_dir, &config, &journal_path);
        cleanup_archive_if_due(&config, &mut last_archive_cleanup, true);
        drain_events(&receiver);
    }

    let mut last_event: Option<Instant> = None;
    let mut last_poll = Instant::now();
    let mut last_config_refresh = Instant::now();
    let mut last_daily_key: Option<String> = None;

    while !stop_signal.load(Ordering::Relaxed) {
        match receiver.recv_timeout(POLL_TICK) {
            Ok(_) => {
                // Настройки могли измениться в окне настроек — читаем заново.
                config = reload_config(&config_path, &config);
                if config.enabled {
                    last_event = Some(Instant::now());
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        // Периодически подхватываем переключение автоочистки из трея,
        // даже если на рабочем столе не было событий.
        if last_config_refresh.elapsed() >= CONFIG_REFRESH_INTERVAL {
            last_config_refresh = Instant::now();
            config = reload_config(&config_path, &config);
        }

        if !config.enabled {
            last_event = None;
        }

        // Раз в час проверяем срок хранения архива независимо от таймера уборки.
        cleanup_archive_if_due(&config, &mut last_archive_cleanup, false);

        match config.timer.clone() {
            Timer::Immediate => {
                let debounce_elapsed = last_event.is_some_and(|at| at.elapsed() >= DEBOUNCE);
                if debounce_elapsed {
                    last_event = None;
                    config = reload_config(&config_path, &config);

                    if config.enabled && matches!(&config.timer, Timer::Immediate) {
                        run_and_log(&desktop_dir, &config, &journal_path);
                        cleanup_archive_if_due(&config, &mut last_archive_cleanup, true);
                    }

                    // События от собственных перемещений не должны запускать новый проход.
                    drain_events(&receiver);
                    last_poll = Instant::now();
                }
            }
            Timer::Minutes(_) => {
                last_event = None;

                if last_poll.elapsed() >= MINUTES_POLL_INTERVAL {
                    last_poll = Instant::now();
                    config = reload_config(&config_path, &config);

                    if let Timer::Minutes(minutes) = &config.timer {
                        if !config.enabled {
                            continue;
                        }
                        let min_age = Duration::from_secs(minutes.saturating_mul(60));
                        if let Err(err) = run_cleanup_filtered(
                            &desktop_dir,
                            &config,
                            &journal_path,
                            Some(min_age),
                        ) {
                            eprintln!("desktop_cleaner: уборка по таймеру не удалась: {err}");
                        }
                        cleanup_archive_if_due(&config, &mut last_archive_cleanup, true);
                    }
                }
            }
            Timer::Daily(_) => {
                last_event = None;

                if last_poll.elapsed() >= DAILY_POLL_INTERVAL {
                    last_poll = Instant::now();
                    config = reload_config(&config_path, &config);

                    if !config.enabled {
                        continue;
                    }

                    if let Timer::Daily(schedule) = &config.timer {
                        let now = Local::now();
                        let minute_key = now.format("%Y-%m-%d %H:%M").to_string();
                        let is_time = now.format("%H:%M").to_string() == *schedule;

                        if is_time && last_daily_key.as_deref() != Some(minute_key.as_str()) {
                            last_daily_key = Some(minute_key);
                            run_and_log(&desktop_dir, &config, &journal_path);
                            cleanup_archive_if_due(&config, &mut last_archive_cleanup, true);
                        }
                    }
                }
            }
        }
    }
}

/// Общая реализация прохода с необязательным фильтром по возрасту.
fn run_cleanup_filtered(
    desktop_dir: &Path,
    config: &Config,
    journal_path: &Path,
    min_age: Option<Duration>,
) -> io::Result<usize> {
    let mut journal = Journal::load_from(journal_path)?;
    let mut moved: Vec<MovedItem> = Vec::new();

    for entry in fs::read_dir(desktop_dir)? {
        let entry = entry?;
        let path = entry.path();

        if !should_archive(&path, config) {
            continue;
        }

        if let Some(min_age) = min_age {
            match entry.metadata().and_then(|metadata| metadata.modified()) {
                Ok(modified) => {
                    let age = SystemTime::now()
                        .duration_since(modified)
                        .unwrap_or(Duration::ZERO);
                    if age < min_age {
                        continue;
                    }
                }
                Err(_) => continue,
            }
        }

        match move_to_archive(&path, config) {
            Ok(destination) => moved.push(MovedItem {
                from: path,
                to: destination,
            }),
            // Объект исчез между чтением каталога и переносом — не ошибка.
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                save_moved(&mut journal, moved, journal_path)?;
                return Err(err);
            }
        }
    }

    let count = moved.len();
    save_moved(&mut journal, moved, journal_path)?;

    // Finder иногда оставляет пустые «дыры» на месте убранных файлов.
    // Просим его выровнять значки, но только для настоящего Рабочего стола,
    // чтобы не трогать Finder во время тестов с временными папками.
    #[cfg(target_os = "macos")]
    if count > 0 && is_real_desktop(desktop_dir) {
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                "tell application \"Finder\" to clean up window of desktop",
            ])
            .spawn();
    }

    Ok(count)
}

#[cfg(target_os = "macos")]
fn is_real_desktop(desktop_dir: &Path) -> bool {
    let Some(real_desktop) = dirs::desktop_dir() else {
        return false;
    };

    if real_desktop == desktop_dir {
        return true;
    }

    match (fs::canonicalize(&real_desktop), fs::canonicalize(desktop_dir)) {
        (Ok(real), Ok(current)) => real == current,
        _ => false,
    }
}

fn save_moved(
    journal: &mut Journal,
    moved: Vec<MovedItem>,
    journal_path: &Path,
) -> io::Result<()> {
    if moved.is_empty() {
        return Ok(());
    }

    journal.record_batch(moved);
    journal.save_to(journal_path)
}

fn reload_config(config_path: &Path, fallback: &Config) -> Config {
    Config::load_from(config_path).unwrap_or_else(|_| fallback.clone())
}

/// Отправляет устаревшие объекты архива в Корзину, если включено автоудаление.
fn cleanup_archive_if_due(config: &Config, last_run: &mut Instant, force: bool) {
    if !config.enabled {
        return;
    }

    let Some(days) = config.auto_delete_days else {
        return;
    };

    if !force && last_run.elapsed() < ARCHIVE_CLEANUP_INTERVAL {
        return;
    }

    *last_run = Instant::now();

    match cleanup_expired_archive(&config.archive_dir, days) {
        Ok(0) => {}
        Ok(count) => {
            eprintln!("desktop_cleaner: отправлено в Корзину из архива: {count}");
        }
        Err(err) => {
            eprintln!("desktop_cleaner: ошибка очистки архива: {err}");
        }
    }
}

fn run_and_log(desktop_dir: &Path, config: &Config, journal_path: &Path) {
    if let Err(err) = run_cleanup_once(desktop_dir, config, journal_path) {
        eprintln!("desktop_cleaner: уборка не удалась: {err}");
    }
}

fn drain_events(receiver: &Receiver<notify::Result<notify::Event>>) {
    while receiver.try_recv().is_ok() {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CleanMode, SortMode};

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parents");
        }
        fs::write(path, contents).expect("write file");
    }

    fn config(archive_dir: PathBuf, mode: CleanMode) -> Config {
        Config {
            mode,
            archive_dir,
            sort_mode: SortMode::ByCategory,
            ..Config::default()
        }
    }

    #[test]
    fn cleanup_touches_only_top_level_entries() {
        let root = tempfile::tempdir().expect("temp dir");
        let desktop = root.path().join("Desktop");
        let archive = root.path().join("Archive");
        let journal_path = root.path().join("journal.json");

        write(&desktop.join("note.txt"), "top");
        write(&desktop.join("Project").join("notes.txt"), "nested");

        let cfg = config(archive.clone(), CleanMode::OnlyFolders);
        let count = run_cleanup_once(&desktop, &cfg, &journal_path).expect("cleanup");

        assert_eq!(count, 1, "убран должен быть только файл верхнего уровня");
        assert!(!desktop.join("note.txt").exists());
        assert!(
            desktop.join("Project").join("notes.txt").exists(),
            "вложенный файл в оставленной папке трогать нельзя"
        );
        assert_eq!(
            fs::read_to_string(desktop.join("Project").join("notes.txt")).unwrap(),
            "nested"
        );
        assert_eq!(
            fs::read_to_string(archive.join("Documents").join("note.txt")).unwrap(),
            "top"
        );

        let journal = Journal::load_from(&journal_path).expect("journal");
        assert_eq!(journal.batches.len(), 1);
        assert_eq!(journal.batches[0].items.len(), 1);
        assert_eq!(journal.batches[0].items[0].from, desktop.join("note.txt"));
    }

    #[test]
    fn cleanup_moves_folder_as_a_single_item() {
        let root = tempfile::tempdir().expect("temp dir");
        let desktop = root.path().join("Desktop");
        let archive = root.path().join("Archive");
        let journal_path = root.path().join("journal.json");

        write(&desktop.join("loose.txt"), "loose");
        write(&desktop.join("Data").join("inner").join("deep.txt"), "deep");

        let cfg = config(archive.clone(), CleanMode::Nothing);
        let count = run_cleanup_once(&desktop, &cfg, &journal_path).expect("cleanup");

        assert_eq!(count, 2, "папка считается одним объектом, не её содержимое");
        assert!(archive.join("Other").join("Data").join("inner").join("deep.txt").exists());
        assert!(archive.join("Documents").join("loose.txt").exists());

        let journal = Journal::load_from(&journal_path).expect("journal");
        assert_eq!(journal.batches[0].items.len(), 2);
    }

    #[test]
    fn cleanup_selected_formats_moves_only_matching_files() {
        let root = tempfile::tempdir().expect("temp dir");
        let desktop = root.path().join("Desktop");
        let archive = root.path().join("Archive");
        let journal_path = root.path().join("journal.json");

        write(&desktop.join("photo.png"), "png");
        write(&desktop.join("slides.pptx"), "pptx");
        write(&desktop.join("Project").join("notes.txt"), "nested");

        let cfg = config(
            archive.clone(),
            CleanMode::SelectedFormats(vec!["png".into()]),
        );
        let count = run_cleanup_once(&desktop, &cfg, &journal_path).expect("cleanup");

        assert_eq!(count, 1);
        assert!(archive.join("Images").join("photo.png").exists());
        assert!(
            desktop.join("slides.pptx").exists(),
            "невыбранные форматы должны остаться на столе"
        );
        assert!(
            desktop.join("Project").join("notes.txt").exists(),
            "папки не должны затрагиваться"
        );

        let journal = Journal::load_from(&journal_path).expect("journal");
        assert_eq!(journal.batches[0].items.len(), 1);
    }

    #[test]
    fn manual_cleanup_works_even_when_auto_is_disabled() {
        let root = tempfile::tempdir().expect("temp dir");
        let desktop = root.path().join("Desktop");
        let archive = root.path().join("Archive");
        let journal_path = root.path().join("journal.json");
        write(&desktop.join("note.txt"), "manual");

        let mut cfg = config(archive.clone(), CleanMode::OnlyFolders);
        cfg.enabled = false;

        let count = run_cleanup_once(&desktop, &cfg, &journal_path).expect("cleanup");

        assert_eq!(count, 1);
        assert!(archive.join("Documents").join("note.txt").exists());
    }

    #[test]
    fn cleanup_with_nothing_to_move_does_not_create_journal() {
        let root = tempfile::tempdir().expect("temp dir");
        let desktop = root.path().join("Desktop");
        let journal_path = root.path().join("journal.json");
        fs::create_dir_all(&desktop).unwrap();
        write(&desktop.join(".DS_Store"), "hidden");

        let cfg = config(root.path().join("Archive"), CleanMode::Nothing);
        let count = run_cleanup_once(&desktop, &cfg, &journal_path).expect("cleanup");

        assert_eq!(count, 0);
        assert!(desktop.join(".DS_Store").exists());
        assert!(!journal_path.exists());
    }
}
