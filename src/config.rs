//! Конфигурация очистки рабочего стола.
//!
//! Настройки хранятся в `config.json` рядом с рабочим каталогом приложения
//! и загружаются/сохраняются методами [`Config::load`] и [`Config::save`].

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Имя файла конфигурации по умолчанию.
pub const CONFIG_FILE: &str = "config.json";

/// Что убирать с рабочего стола.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanMode {
    /// Убрать всё (пустой стол).
    Nothing,
    /// Убрать только скриншоты по префиксам.
    OnlyScreenshots,
    /// Убрать все отдельные файлы, папки оставить на месте.
    OnlyFolders,
    /// Убрать только файлы указанных форматов (расширений).
    SelectedFormats(Vec<String>),
}

/// Когда запускать очистку.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Timer {
    /// Сразу при запуске.
    Immediate,
    /// Раз в N минут.
    Minutes(u64),
    /// Каждый день в указанное время, например `"03:30"`.
    Daily(String),
}

/// Как сортировать перемещаемые файлы в архиве.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortMode {
    /// По дате изменения.
    ByDate,
    /// По категории (документы, изображения и т.п.).
    ByCategory,
}

/// Язык интерфейса.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    /// English.
    En,
    /// Русский.
    Ru,
}

/// Полный набор настроек приложения.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Автоматическая уборка включена (главный тумблер).
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Запускать при входе в систему и держать агент живым.
    #[serde(default = "default_true")]
    pub launch_at_login: bool,
    /// Язык интерфейса (`None` — определить по языку системы).
    #[serde(default)]
    pub language: Option<Language>,
    /// Режим очистки.
    pub mode: CleanMode,
    /// Имена или маски, которые нельзя перемещать.
    pub exceptions: Vec<String>,
    /// Префиксы имён, по которым распознаются скриншоты.
    pub screenshot_prefixes: Vec<String>,
    /// Расписание очистки.
    pub timer: Timer,
    /// Каталог архива.
    pub archive_dir: PathBuf,
    /// Через сколько дней удалять объекты из архива в Корзину (`None` — никогда).
    #[serde(default)]
    pub auto_delete_days: Option<u32>,
    /// Режим сортировки в архиве.
    pub sort_mode: SortMode,
}

fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            launch_at_login: true,
            language: None,
            mode: CleanMode::Nothing,
            exceptions: Vec::new(),
            screenshot_prefixes: vec![
                "Снимок экрана".to_string(),
                "Screen Shot".to_string(),
            ],
            timer: Timer::Immediate,
            archive_dir: default_archive_dir(),
            auto_delete_days: None,
            sort_mode: SortMode::ByDate,
        }
    }
}

/// `~/Documents/Desktop_Archive` (с разумными запасными вариантами).
pub fn default_archive_dir() -> PathBuf {
    dirs::document_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Documents")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Desktop_Archive")
}

/// Определяет язык системы macOS по `AppleLocale` / `LANG`.
///
/// Если локаль начинается с `ru` — русский, иначе английский.
pub fn detect_system_language() -> Language {
    let locale = std::env::var("AppleLocale")
        .or_else(|_| std::env::var("LANG"))
        .or_else(|_| std::env::var("LC_ALL"))
        .unwrap_or_default();
    let locale = locale.trim().to_lowercase();

    if locale.starts_with("ru") {
        Language::Ru
    } else {
        Language::En
    }
}

impl Config {
    /// Язык интерфейса с учётом автоопределения для старых конфигов.
    pub fn resolved_language(&self) -> Language {
        self.language.unwrap_or_else(detect_system_language)
    }

    /// Загрузить настройки из `config.json`.
    ///
    /// Если файла ещё нет, возвращается конфигурация по умолчанию.
    pub fn load() -> io::Result<Self> {
        Self::load_from(CONFIG_FILE)
    }

    /// Сохранить настройки в `config.json`.
    pub fn save(&self) -> io::Result<()> {
        self.save_to(CONFIG_FILE)
    }

    /// Загрузить настройки из произвольного пути (удобно для тестов).
    pub fn load_from(path: impl AsRef<Path>) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).map_err(invalid_data),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err),
        }
    }

    /// Сохранить настройки в произвольный путь (удобно для тестов).
    pub fn save_to(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(invalid_data)?;
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)
    }
}

fn invalid_data(err: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

const LAUNCH_AGENT_LABEL: &str = "com.local.desktopcleaner";

/// Путь к LaunchAgent plist в домашней библиотеке пользователя.
pub fn launch_agent_plist_path() -> io::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "нет домашней папки")
    })?;
    Ok(home
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist")))
}

/// Путь к исполняемому файлу для LaunchAgent.
pub fn launch_agent_program_path() -> PathBuf {
    let installed = PathBuf::from("/Applications/Desktop Cleaner.app/Contents/MacOS/desktop_cleaner");
    if installed.exists() {
        return installed;
    }
    std::env::current_exe().unwrap_or(installed)
}

/// Синхронизирует LaunchAgent: записывает plist и включает/выключает агент.
///
/// `SuccessfulExit=false` в KeepAlive означает: macOS перезапускает процесс
/// после падения/выгрузки, но не перезапускает после обычного exit(0)
/// (например, когда пользователь нажал «Выход» в трее).
pub fn sync_launch_agent(enabled: bool) -> io::Result<()> {
    let plist_path = launch_agent_plist_path()?;
    if let Some(parent) = plist_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let program = launch_agent_program_path();
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{program}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ProcessType</key>
    <string>Interactive</string>
</dict>
</plist>
"#,
        label = LAUNCH_AGENT_LABEL,
        program = xml_escape(&program.to_string_lossy()),
    );
    fs::write(&plist_path, plist)?;

    let uid = current_uid()?;
    let domain = format!("gui/{uid}");
    let plist_str = plist_path.to_string_lossy().to_string();

    if enabled {
        // Не делаем bootout на удачном пути: если агент уже загружен, это
        // может перезапустить текущий процесс. Bootstrap идемпотентно
        // обновляем через bootout+bootstrap только при необходимости.
        let bootstrapped = Command::new("launchctl")
            .args(["bootstrap", &domain, &plist_str])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if bootstrapped {
            return Ok(());
        }

        let _ = Command::new("launchctl")
            .args(["bootout", &domain, &plist_str])
            .status();
        let _ = Command::new("launchctl")
            .args(["unload", &plist_str])
            .status();

        let status = Command::new("launchctl")
            .args(["bootstrap", &domain, &plist_str])
            .status();
        match status {
            Ok(s) if s.success() => Ok(()),
            _ => {
                let fallback = Command::new("launchctl")
                    .args(["load", &plist_str])
                    .status()?;
                if fallback.success() {
                    Ok(())
                } else {
                    Err(io::Error::other("не удалось включить LaunchAgent"))
                }
            }
        }
    } else {
        let _ = Command::new("launchctl")
            .args(["bootout", &domain, &plist_str])
            .status();
        let _ = Command::new("launchctl")
            .args(["unload", &plist_str])
            .status();
        Ok(())
    }
}

fn current_uid() -> io::Result<String> {
    let output = Command::new("id").arg("-u").output()?;
    if !output.status.success() {
        return Err(io::Error::other("не удалось получить uid"));
    }
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if uid.is_empty() {
        return Err(io::Error::other("пустой uid"));
    }
    Ok(uid)
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_expected_values() {
        let config = Config::default();

        assert_eq!(config.mode, CleanMode::Nothing);
        assert_eq!(config.timer, Timer::Immediate);
        assert_eq!(config.sort_mode, SortMode::ByDate);
        assert_eq!(config.language, None);
        assert!(config.launch_at_login);
        assert!(config.exceptions.is_empty());
        assert_eq!(
            config.screenshot_prefixes,
            vec!["Снимок экрана".to_string(), "Screen Shot".to_string()]
        );
        assert_eq!(
            config.archive_dir.file_name().and_then(|name| name.to_str()),
            Some("Desktop_Archive")
        );
    }

    #[test]
    fn save_and_load_roundtrip() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.json");

        let config = Config {
            enabled: true,
            launch_at_login: false,
            language: Some(Language::Ru),
            mode: CleanMode::SelectedFormats(vec!["pdf".into(), "png".into()]),
            exceptions: vec!["important.docx".into()],
            screenshot_prefixes: vec!["Screenshot".into()],
            timer: Timer::Daily("03:30".into()),
            archive_dir: dir.path().join("archive"),
            auto_delete_days: Some(7),
            sort_mode: SortMode::ByCategory,
        };

        config.save_to(&path).expect("save");
        let loaded = Config::load_from(&path).expect("load");

        assert_eq!(loaded, config);
        assert!(Config::load_from(dir.path().join("missing.json")).is_ok());
    }

    #[test]
    fn missing_new_fields_default_for_legacy_config() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.json");

        Config::default().save_to(&path).expect("save");
        let mut value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("enabled");
        object.remove("auto_delete_days");
        object.remove("language");
        object.remove("launch_at_login");
        fs::write(&path, serde_json::to_string_pretty(&value).unwrap()).unwrap();

        let loaded = Config::load_from(&path).expect("load");
        assert!(loaded.enabled);
        assert!(loaded.launch_at_login);
        assert_eq!(loaded.auto_delete_days, None);
        assert_eq!(loaded.language, None);
    }
}
