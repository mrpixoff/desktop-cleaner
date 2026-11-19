//! Правила отбора: какие объекты рабочего стола нужно отправить в архив.

use crate::config::{CleanMode, Config};
use std::path::Path;
use unicode_normalization::UnicodeNormalization;

/// Приводит строку к NFC: macOS отдаёт имена файлов в NFD
/// (например, «й» = «и» + U+0306), поэтому сравнивать нужно нормализованные формы.
fn normalize(text: &str) -> String {
    text.nfc().collect()
}

/// Решает, нужно ли перемещать `path` в архив.
///
/// Приоритет проверок (первое совпадение выигрывает):
/// 1. скрытые системные файлы macOS — никогда;
/// 2. имена из [`Config::exceptions`] — никогда;
/// 3. остальное зависит от [`Config::mode`]:
///    - `Nothing` — убираем всё (пустой стол);
///    - `OnlyScreenshots` — только скриншоты по префиксам;
///    - `OnlyFolders` — все отдельные файлы, папки остаются;
///    - `SelectedFormats` — только файлы указанных расширений.
pub fn should_archive(path: &Path, config: &Config) -> bool {
    let raw_name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    let name = normalize(raw_name.as_ref());

    // Скрытые файлы и служебные файлы Finder трогать нельзя ни в каком режиме.
    if is_hidden_system_file(&name) {
        return false;
    }

    // Исключения пользователя имеют приоритет над любым режимом.
    if config
        .exceptions
        .iter()
        .any(|exception| normalize(exception) == name)
    {
        return false;
    }

    match &config.mode {
        CleanMode::Nothing => true,
        CleanMode::OnlyScreenshots => {
            !path.is_dir()
                && config.screenshot_prefixes.iter().any(|prefix| {
                    let prefix = normalize(prefix);
                    !prefix.is_empty() && name.starts_with(&prefix)
                })
        }
        CleanMode::OnlyFolders => !path.is_dir(),
        CleanMode::SelectedFormats(formats) => {
            !path.is_dir()
                && path
                    .extension()
                    .map(|ext| {
                        let ext = ext.to_string_lossy();
                        formats.iter().any(|format| {
                            format
                                .trim_start_matches('.')
                                .eq_ignore_ascii_case(ext.as_ref())
                        })
                    })
                    .unwrap_or(false)
        }
    }
}

/// Скрытые (начинающиеся с точки) и служебные файлы macOS.
fn is_hidden_system_file(name: &str) -> bool {
    name.starts_with('.') || name == ".DS_Store" || name == ".localized"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CleanMode, Config};

    fn config(mode: CleanMode) -> Config {
        Config {
            mode,
            ..Config::default()
        }
    }

    #[test]
    fn hidden_files_and_exceptions_win_over_mode() {
        let everything = config(CleanMode::Nothing);

        // Скрытые системные файлы не трогаем даже в самом агрессивном режиме.
        assert!(!should_archive(Path::new(".DS_Store"), &everything));
        assert!(!should_archive(Path::new(".localized"), &everything));
        assert!(!should_archive(Path::new(".gitignore"), &everything));

        // Исключение пользователя сильнее режима.
        let with_exception = Config {
            mode: CleanMode::Nothing,
            exceptions: vec!["keep-me.txt".into()],
            ..Config::default()
        };
        assert!(!should_archive(Path::new("keep-me.txt"), &with_exception));
        assert!(should_archive(Path::new("other.txt"), &with_exception));
    }

    #[test]
    fn nothing_mode_archives_everything_not_excluded() {
        let cfg = config(CleanMode::Nothing);
        assert!(should_archive(Path::new("report.pdf"), &cfg));
        assert!(should_archive(Path::new("Projects"), &cfg));
    }

    #[test]
    fn only_folders_keeps_folders_and_archives_files() {
        let dir = tempfile::tempdir().expect("temp dir");
        let folder = dir.path().join("Projects");
        let file = dir.path().join("notes.txt");
        std::fs::create_dir(&folder).expect("create dir");
        std::fs::write(&file, b"x").expect("create file");

        let cfg = config(CleanMode::OnlyFolders);
        assert!(!should_archive(&folder, &cfg));
        assert!(should_archive(&file, &cfg));
    }

    #[test]
    fn only_screenshots_archives_only_screenshots() {
        let dir = tempfile::tempdir().expect("temp dir");
        let screenshot = dir.path().join("Снимок экрана 2026-01-01 в 10.00.00.png");
        let screen_shot = dir.path().join("Screen Shot 2026-01-02.png");
        let presentation = dir.path().join("slides.pptx");
        let folder = dir.path().join("Project");
        for path in [&screenshot, &screen_shot, &presentation] {
            std::fs::write(path, b"x").expect("create file");
        }
        std::fs::create_dir(&folder).expect("create dir");

        let cfg = config(CleanMode::OnlyScreenshots);
        assert!(should_archive(&screenshot, &cfg));
        assert!(should_archive(&screen_shot, &cfg));
        assert!(!should_archive(&presentation, &cfg));
        assert!(!should_archive(&folder, &cfg));

        // Свой префикс скриншотов тоже учитывается.
        let custom = Config {
            mode: CleanMode::OnlyScreenshots,
            screenshot_prefixes: vec!["Screenshot".into()],
            ..Config::default()
        };
        assert!(should_archive(Path::new("Screenshot-1.png"), &custom));
        assert!(!should_archive(&screenshot, &custom));
    }

    #[test]
    fn selected_formats_archives_only_listed_extensions() {
        let cfg = config(CleanMode::SelectedFormats(vec!["pdf".into(), "PNG".into()]));

        assert!(should_archive(Path::new("report.pdf"), &cfg));
        assert!(should_archive(Path::new("photo.png"), &cfg)); // без учёта регистра
        assert!(!should_archive(Path::new("archive.zip"), &cfg));
        assert!(!should_archive(Path::new("no-extension"), &cfg));
        assert!(!should_archive(Path::new("Folder"), &cfg));

        // Точка перед расширением в настройках не мешает.
        let dotted = config(CleanMode::SelectedFormats(vec![".md".into()]));
        assert!(should_archive(Path::new("README.md"), &dotted));
        assert!(!should_archive(Path::new("README.txt"), &dotted));
    }

    #[test]
    fn nfd_screenshot_name_matches_nfc_prefix() {
        // macOS хранит имя в NFD. Строка из задачи содержит U+0306 после «Снимок»;
        // сравниваем нормализованные формы и имени, и префикса.
        let nfd_name = "Снимок\u{0306} экрана 2026-01-01.png";
        let cfg = Config {
            mode: CleanMode::OnlyScreenshots,
            screenshot_prefixes: vec!["Снимок\u{0306} экрана".into()],
            ..Config::default()
        };
        assert!(should_archive(Path::new(nfd_name), &cfg));

        // Реальный NFD-случай для «й»: «и» + U+0306 должно совпасть с NFC-префиксом.
        let cfg = Config {
            mode: CleanMode::OnlyScreenshots,
            screenshot_prefixes: vec!["Йошкар-Ола".into()],
            ..Config::default()
        };
        assert!(should_archive(Path::new("И\u{0306}ошкар-Ола 1.png"), &cfg));
    }
}
