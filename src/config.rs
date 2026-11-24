//! Конфигурация очистки рабочего стола.
//!
//! Настройки хранятся в `config.json` рядом с рабочим каталогом приложения
//! и загружаются/сохраняются методами [`Config::load`] и [`Config::save`].

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

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

    if !locale.trim().is_empty() {
        return if is_russian_locale(&locale) {
            Language::Ru
        } else {
            Language::En
        };
    }

    // GUI-приложения, запущенные из Finder/launchd, часто не получают LANG.
    // В этом случае спрашиваем локаль у самой macOS.
    let system_locale = Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_default();

    if is_russian_locale(&system_locale) {
        Language::Ru
    } else {
        Language::En
    }
}

fn is_russian_locale(locale: &str) -> bool {
    locale.trim().to_lowercase().starts_with("ru")
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
    ///
    /// Запись атомарная: сначала во временный файл, затем `rename`.
    /// Это не даёт фоновому наблюдателю прочитать наполовину записанный JSON.
    pub fn save_to(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(invalid_data)?;
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let tmp = temp_sibling(path);
        fs::write(&tmp, json)?;
        if let Err(err) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
        Ok(())
    }
}

fn temp_sibling(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_string());
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    path.with_file_name(format!(".{name}.{}.{nanos}.tmp", std::process::id()))
}

fn invalid_data(err: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err)
}

/// Метка LaunchAgent в системе.
pub const LAUNCH_AGENT_LABEL: &str = "com.local.desktopcleaner";

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
///
/// Для установленного бандла всегда используется `/Applications`, иначе —
/// текущий бинарник (например, при запуске из папки проекта).
pub fn launch_agent_program_path() -> PathBuf {
    let installed = PathBuf::from("/Applications/Desktop Cleaner.app/Contents/MacOS/desktop_cleaner");
    if installed.exists() {
        return installed;
    }
    std::env::current_exe().unwrap_or(installed)
}

/// Реальный статус автозапуска в системе.
///
/// `None` — если macOS не дала узнать статус (тогда UI использует значение
/// из конфигурации). `Some(true)` — plist существует, агент загружен и не
/// отключён через Системные настройки.
pub fn launch_agent_status() -> Option<bool> {
    let plist_path = launch_agent_plist_path().ok()?;
    if !plist_path.exists() {
        return Some(false);
    }

    let loaded = launchctl_loaded(LAUNCH_AGENT_LABEL)?;
    if !loaded {
        return Some(false);
    }

    // Пользователь мог выключить автозапуск в Системных настройках:
    // launchd при этом помечает сервис как disabled, не выгружая его.
    Some(!launchctl_disabled(LAUNCH_AGENT_LABEL).unwrap_or(false))
}

/// Удобная обёртка для мест, где нужен только `bool`.
pub fn launch_agent_enabled() -> bool {
    launch_agent_status().unwrap_or(false)
}

/// Синхронизирует LaunchAgent: записывает plist и включает/выключает агент.
///
/// `SuccessfulExit=false` в KeepAlive означает: macOS перезапускает процесс
/// после падения/выгрузки, но не перезапускает после обычного exit(0)
/// (например, когда пользователь нажал «Выход» в трее).
///
/// При выключении мы **не** делаем `bootout`: он завершил бы текущий
/// запущенный процесс. Вместо этого сервис помечается disabled и plist
/// удаляется — при следующем входе приложение уже не запустится.
pub fn sync_launch_agent(enabled: bool) -> io::Result<()> {
    let plist_path = launch_agent_plist_path()?;
    let program = launch_agent_program_path();
    sync_launch_agent_at(LAUNCH_AGENT_LABEL, &plist_path, &program, enabled)
}

fn sync_launch_agent_at(
    label: &str,
    plist_path: &Path,
    program: &Path,
    enabled: bool,
) -> io::Result<()> {
    if let Some(parent) = plist_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let uid = current_uid()?;
    let domain = format!("gui/{uid}");
    let service = format!("{domain}/{label}");
    let plist_str = plist_path.to_string_lossy().into_owned();

    if !enabled {
        let _ = run_launchctl(&["disable", &service]);
        match fs::remove_file(plist_path) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        }
    }

    fs::write(plist_path, launch_agent_plist(label, program))?;

    // Снимаем возможный disabled-флаг, выставленный Системными настройками
    // или предыдущим выключением автозапуска.
    let _ = run_launchctl(&["enable", &service]);

    // Если агент уже загружен (типичный случай при старте через launchd),
    // не перезагружаем его: bootout убил бы текущий процесс. Новый plist
    // всё равно вступит в силу при следующем входе в систему.
    if launchctl_loaded(label) == Some(true) {
        return Ok(());
    }

    let _ = run_launchctl(&["bootstrap", &domain, &plist_str]);
    if launchctl_loaded(label) == Some(true) {
        return Ok(());
    }

    // Fallback для старых версий macOS.
    let _ = run_launchctl(&["load", &plist_str]);
    if launchctl_loaded(label) == Some(true) {
        return Ok(());
    }

    Err(io::Error::other("не удалось включить LaunchAgent"))
}

fn launch_agent_plist(label: &str, program: &Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{program}</string>
        <string>--background</string>
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
        label = xml_escape(label),
        program = xml_escape(&program.to_string_lossy()),
    )
}

fn launchctl_loaded(label: &str) -> Option<bool> {
    let uid = current_uid().ok()?;
    let service = format!("gui/{uid}/{label}");
    run_launchctl(&["print", &service])
}

fn launchctl_disabled(label: &str) -> Option<bool> {
    let uid = current_uid().ok()?;
    let domain = format!("gui/{uid}");
    let output = Command::new("launchctl")
        .args(["print-disabled", &domain])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    Some(parse_disabled_services(
        &String::from_utf8_lossy(&output.stdout),
        label,
    ))
}

fn parse_disabled_services(output: &str, label: &str) -> bool {
    let needle = format!("\"{label}\"");
    output
        .lines()
        .any(|line| line.contains(&needle) && line.contains("disabled"))
}

fn run_launchctl(args: &[&str]) -> Option<bool> {
    Command::new("launchctl")
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()
        .map(|status| status.success())
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

    #[test]
    fn russian_locale_detection_handles_common_forms() {
        assert!(is_russian_locale("ru_RU"));
        assert!(is_russian_locale("RU"));
        assert!(is_russian_locale(" ru-RU "));
        assert!(!is_russian_locale("en_US"));
        assert!(!is_russian_locale(""));
    }

    #[test]
    fn launch_agent_plist_has_required_keys_and_background_flag() {
        let plist = launch_agent_plist(
            "com.local.desktopcleaner",
            Path::new("/Applications/Desktop Cleaner.app/Contents/MacOS/desktop_cleaner"),
        );

        assert!(plist.contains("<key>Label</key>"));
        assert!(plist.contains("<string>com.local.desktopcleaner</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(plist.contains("<key>KeepAlive</key>"));
        assert!(plist.contains("<key>SuccessfulExit</key>"));
        assert!(plist.contains("<false/>"));
        assert!(plist.contains("<string>--background</string>"));
        assert!(plist.contains("/Applications/Desktop Cleaner.app/Contents/MacOS/desktop_cleaner"));
    }

    #[test]
    fn launch_agent_plist_escapes_program_path() {
        let plist = launch_agent_plist("com.test", Path::new("/tmp/A&B <test>.app"));

        assert!(plist.contains("/tmp/A&amp;B &lt;test&gt;.app"));
        assert!(!plist.contains("A&B"));
    }

    #[test]
    fn disabled_services_parser_distinguishes_enabled_and_disabled() {
        let output = "\tdisabled services = {\n\
            \t\t\"com.local.desktopcleaner\" => disabled\n\
            \t\t\"com.example.other\" => enabled\n\
            \t}";
        assert!(parse_disabled_services(output, "com.local.desktopcleaner"));
        assert!(!parse_disabled_services(output, "com.example.other"));
        assert!(!parse_disabled_services(output, "com.example.missing"));

        let enabled_only = "\t\t\"com.local.desktopcleaner\" => enabled\n";
        assert!(!parse_disabled_services(
            enabled_only,
            "com.local.desktopcleaner"
        ));
    }

    #[test]
    fn atomic_save_leaves_no_temp_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("config.json");

        Config::default().save_to(&path).expect("save");
        Config::default().save_to(&path).expect("save again");

        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "временные файлы не должны оставаться");
        assert!(Config::load_from(&path).is_ok());
    }
}
