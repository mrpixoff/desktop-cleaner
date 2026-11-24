//! Точка входа: менюбар macOS + фоновый наблюдатель, а окно настроек
//! запускается отдельным процессом (`--settings`), чтобы не держать память.

use desktop_cleaner::engine::start_watcher;
use desktop_cleaner::ui;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
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
    let pid_path = data_dir.join("app.pid");
    let desktop_dir = dirs::desktop_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "нет домашней папки"))?;

    // Дочерний процесс: только окно настроек, без менюбара.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--settings") {
        ui::run_settings_window(config_path, journal_path, desktop_dir)?;
        return Ok(());
    }
    let background = args.iter().any(|arg| arg == "--background");

    // Single instance: если фоновый процесс уже жив, не запускаем второй watcher.
    // При старте через launchd (`--background`) тихо выходим, чтобы включение
    // автозапуска из настроек не открывало лишнее окно.
    let Some(_pid_guard) = PidGuard::acquire(&pid_path)? else {
        if !background {
            if let Err(err) = ui::spawn_settings_window() {
                eprintln!("desktop_cleaner: не удалось открыть настройки: {err}");
            }
        }
        return Ok(());
    };

    let config_existed = config_path.exists();
    let mut startup_config =
        desktop_cleaner::config::Config::load_from(&config_path).unwrap_or_default();

    if !config_existed {
        // Первый запуск: ставим агент, если так настроено по умолчанию.
        if startup_config.launch_at_login {
            if let Err(err) = desktop_cleaner::config::sync_launch_agent(true) {
                eprintln!("desktop_cleaner: не удалось установить LaunchAgent: {err}");
            }
        }
    } else if let Some(actual) = desktop_cleaner::config::launch_agent_status() {
        // Состояние галочки должно совпадать с реальным положением в системе:
        // пользователь мог изменить автозапуск через Системные настройки.
        if actual != startup_config.launch_at_login {
            startup_config.launch_at_login = actual;
            if let Err(err) = startup_config.save_to(&config_path) {
                eprintln!("desktop_cleaner: не удалось сохранить статус автозапуска: {err}");
            }
        }
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

/// Проверяет, жив ли уже основной фоновый процесс по PID-файлу.
fn another_instance_running(pid_path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(pid_path) else {
        return false;
    };

    let Ok(pid) = contents.trim().parse::<u32>() else {
        return false;
    };

    if pid == std::process::id() || !process_alive(pid) {
        return false;
    }

    // PID мог быть переиспользован другим процессом — не блокируем запуск.
    process_is_desktop_cleaner(pid)
}

fn process_alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn process_is_desktop_cleaner(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output();

    match output {
        Ok(output) if output.status.success() => {
            let comm = String::from_utf8_lossy(&output.stdout);
            let comm = comm.trim();
            comm.contains("desktop_cleaner") || comm.contains("Desktop Cleaner")
        }
        // Если `ps` недоступен, лучше перестраховаться и не запускать второй watcher.
        _ => true,
    }
}

/// Удаляет PID-файл при выходе основного процесса.
struct PidGuard {
    path: PathBuf,
    pid: u32,
}

impl PidGuard {
    /// Атомарно занимает PID-файл. `Ok(None)` — уже работает другой экземпляр.
    fn acquire(path: &Path) -> io::Result<Option<Self>> {
        if another_instance_running(path) {
            return Ok(None);
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let pid = std::process::id();
        for _ in 0..2 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(mut file) => {
                    if let Err(err) = file.write_all(pid.to_string().as_bytes()) {
                        let _ = fs::remove_file(path);
                        return Err(err);
                    }
                    return Ok(Some(Self {
                        path: path.to_path_buf(),
                        pid,
                    }));
                }
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    // Первый процесс мог только что создать файл и ещё не успеть
                    // записать PID — даём ему несколько миллисекунд.
                    for _ in 0..10 {
                        if another_instance_running(path) {
                            return Ok(None);
                        }
                        thread::sleep(Duration::from_millis(20));
                    }

                    // Файл остался от упавшего процесса — забираем его себе.
                    let _ = fs::remove_file(path);
                }
                Err(err) => return Err(err),
            }
        }

        Ok(None)
    }
}

impl Drop for PidGuard {
    fn drop(&mut self) {
        // Удаляем только свой PID-файл, чтобы не снести чужой при гонке.
        let Ok(contents) = fs::read_to_string(&self.path) else {
            return;
        };
        if contents.trim().parse::<u32>().ok() == Some(self.pid) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
