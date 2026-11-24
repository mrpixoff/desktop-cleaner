//! Менюбар macOS и компактное окно настроек в стиле Vorssaint.

use crate::config::{
    default_archive_dir, launch_agent_status, sync_launch_agent, CleanMode, Config, Language,
    SortMode, Timer,
};
use crate::engine::run_cleanup_once;
use crate::journal::undo_last_batch;
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, FontId, Margin, RichText, Rounding, Sense, Stroke, Ui,
    ViewportCommand,
};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tray_icon::menu::{
    CheckMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem,
};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use winit::application::ApplicationHandler;
use winit::event::StartCause;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};

const BG: Color32 = Color32::from_rgb(0x16, 0x17, 0x1A);
const CARD: Color32 = Color32::from_rgb(0x1F, 0x21, 0x26);
const BORDER: Color32 = Color32::from_rgb(0x2C, 0x2F, 0x36);
const FIELD: Color32 = Color32::from_rgb(0x18, 0x1A, 0x1E);
const TEXT: Color32 = Color32::from_rgb(0xE6, 0xE8, 0xEC);
const MUTED: Color32 = Color32::from_rgb(0x8A, 0x8F, 0x98);
const ACCENT: Color32 = Color32::from_rgb(0xB6, 0xF0, 0x5A);
const WARN: Color32 = Color32::from_rgb(0xF0, 0x8A, 0x7A);

const MODE_COUNT: usize = 4;

struct Strings {
    lang: Language,
    subtitle: &'static str,
    auto_title: &'static str,
    auto_on: &'static str,
    auto_off: &'static str,
    launch_at_login: &'static str,
    modes: [&'static str; MODE_COUNT],
    section_clean: &'static str,
    section_exceptions: &'static str,
    section_schedule: &'static str,
    section_archive: &'static str,
    formats_label: &'static str,
    formats_hint: &'static str,
    exceptions_label: &'static str,
    exceptions_hint: &'static str,
    prefixes_label: &'static str,
    prefixes_hint: &'static str,
    timer_immediate: &'static str,
    timer_immediate_hint: &'static str,
    timer_minutes: &'static str,
    timer_daily: &'static str,
    interval: &'static str,
    minutes_word: &'static str,
    time_label: &'static str,
    archive_folder: &'static str,
    choose: &'static str,
    sort_category: &'static str,
    sort_hint: &'static str,
    auto_delete: &'static str,
    retention_label: &'static str,
    retention_hint: &'static str,
    days_suffix: &'static str,
    pause: &'static str,
    enable_auto: &'static str,
    settings: &'static str,
    quit: &'static str,
    undo: &'static str,
    clean_now: &'static str,
    save: &'static str,
    open_archive: &'static str,
    ready: &'static str,
    status_saved: &'static str,
    err_minutes_number: &'static str,
    err_minutes_positive: &'static str,
    err_time_format: &'static str,
    status_auto_on: &'static str,
    status_auto_off: &'static str,
    status_nothing: &'static str,
    status_history_empty: &'static str,
    language_changed: &'static str,
}

const EN: Strings = Strings {
    lang: Language::En,
    subtitle: "Quiet desktop cleanup",
    auto_title: "AUTO-CLEANUP",
    auto_on: "Automatic cleanup: On",
    auto_off: "Automatic cleanup: Paused",
    launch_at_login: "Launch at Login",
    modes: [
        "Clean everything (empty desktop)",
        "Screenshots only",
        "Everything except folders",
        "Selected formats",
    ],
    section_clean: "WHAT TO CLEAN",
    section_exceptions: "EXCEPTIONS AND SCREENSHOTS",
    section_schedule: "SCHEDULE",
    section_archive: "ARCHIVE AND HISTORY",
    formats_label: "Comma-separated formats",
    formats_hint: "pdf, png, docx",
    exceptions_label: "Exceptions, one per line",
    exceptions_hint: "important.docx",
    prefixes_label: "Screenshot prefixes",
    prefixes_hint: "Screen Shot\nСнимок экрана",
    timer_immediate: "Archive new files immediately",
    timer_immediate_hint: "Moves new files 3 seconds after they appear.",
    timer_minutes: "Every N minutes",
    timer_daily: "Every day",
    interval: "Interval",
    minutes_word: "minutes",
    time_label: "Time",
    archive_folder: "Archive folder",
    choose: "Choose…",
    sort_category: "Sort by category",
    sort_hint: "Off — by date. History keeps the last 50 cleanups.",
    auto_delete: "Auto-delete from archive (to Trash)",
    retention_label: "Archive retention",
    retention_hint: "Old items are moved to the macOS Trash.",
    days_suffix: "d.",
    pause: "Pause",
    enable_auto: "Enable auto-cleanup",
    settings: "Settings…",
    quit: "Quit",
    undo: "Undo last",
    clean_now: "Clean by rules",
    save: "Save and hide to background",
    open_archive: "Open archive",
    ready: "Ready.",
    status_saved: "Saved. The window will close; cleanup continues in the background.",
    err_minutes_number: "Interval must be a number of minutes",
    err_minutes_positive: "Interval must be greater than zero",
    err_time_format: "Time must be in HH:MM format",
    status_auto_on: "Auto-cleanup enabled.",
    status_auto_off: "Auto-cleanup paused.",
    status_nothing: "Nothing to move — the desktop is already clean.",
    status_history_empty: "History is empty.",
    language_changed: "Language: English.",
};

const RU: Strings = Strings {
    lang: Language::Ru,
    subtitle: "Тихая уборка рабочего стола",
    auto_title: "АВТООЧИСТКА",
    auto_on: "Автоматическая уборка: Вкл",
    auto_off: "Автоматическая уборка: Пауза",
    launch_at_login: "Запускать при входе в систему",
    modes: [
        "Убрать всё (пустой стол)",
        "Убрать только скриншоты",
        "Убрать всё, кроме папок",
        "Убрать выбранные форматы",
    ],
    section_clean: "ЧТО УБИРАТЬ СО СТОЛА",
    section_exceptions: "ИСКЛЮЧЕНИЯ И СКРИНШОТЫ",
    section_schedule: "РАСПИСАНИЕ",
    section_archive: "АРХИВ И ИСТОРИЯ",
    formats_label: "Форматы через запятую",
    formats_hint: "pdf, png, docx",
    exceptions_label: "Исключения, по одному в строке",
    exceptions_hint: "important.docx",
    prefixes_label: "Префиксы скриншотов",
    prefixes_hint: "Снимок экрана\nScreen Shot",
    timer_immediate: "Автоперенос в архив (сразу)",
    timer_immediate_hint: "Убирает новые файлы через 3 сек. после появления.",
    timer_minutes: "Каждые N минут",
    timer_daily: "Каждый день",
    interval: "Интервал",
    minutes_word: "минут",
    time_label: "Время",
    archive_folder: "Папка архива",
    choose: "Выбрать…",
    sort_category: "Сортировать по категориям",
    sort_hint: "Выкл. — по дате. История хранит последние 50 уборок.",
    auto_delete: "Автоудаление из архива (в Корзину)",
    retention_label: "Срок хранения в архиве",
    retention_hint: "Старые объекты уходят в Корзину macOS.",
    days_suffix: "дн.",
    pause: "Пауза",
    enable_auto: "Включить автоочистку",
    settings: "Настройки…",
    quit: "Выход",
    undo: "Вернуть последнее (Undo)",
    clean_now: "Очистить по правилам",
    save: "Сохранить и свернуть в фон",
    open_archive: "Открыть архив",
    ready: "Готово.",
    status_saved: "Сохранено. Окно закрывается, очистка — в фоне.",
    err_minutes_number: "Интервал должен быть числом минут",
    err_minutes_positive: "Интервал должен быть больше нуля",
    err_time_format: "Время должно быть в формате HH:MM",
    status_auto_on: "Автоочистка включена.",
    status_auto_off: "Автоочистка на паузе.",
    status_nothing: "Нечего переносить — стол уже чистый.",
    status_history_empty: "История пуста.",
    language_changed: "Язык: русский.",
};

fn strings(language: Language) -> &'static Strings {
    match language {
        Language::En => &EN,
        Language::Ru => &RU,
    }
}

fn items_word(language: Language, count: usize) -> &'static str {
    match language {
        Language::En => {
            if count == 1 {
                "item"
            } else {
                "items"
            }
        }
        Language::Ru => objects_word(count),
    }
}

impl Strings {
    fn moved(&self, count: usize, archive: &Path) -> String {
        let items = items_word(self.lang, count);
        match self.lang {
            Language::En => format!("Moved {count} {items} to {}", archive.display()),
            Language::Ru => format!("Перенесено {count} {items} в {}", archive.display()),
        }
    }

    fn restored(&self, count: usize) -> String {
        let items = items_word(self.lang, count);
        match self.lang {
            Language::En => format!("Restored {count} {items} to the Desktop"),
            Language::Ru => format!("Возвращено {count} {items} на Рабочий стол"),
        }
    }

    fn save_error(&self, err: &str) -> String {
        match self.lang {
            Language::En => format!("Could not save: {err}"),
            Language::Ru => format!("Не удалось сохранить: {err}"),
        }
    }

    fn auto_save_error(&self, err: &str) -> String {
        match self.lang {
            Language::En => format!("Could not save auto-cleanup state: {err}"),
            Language::Ru => format!("Не удалось сохранить автоочистку: {err}"),
        }
    }

    fn cleanup_error(&self, err: &str) -> String {
        match self.lang {
            Language::En => format!("Cleanup failed: {err}"),
            Language::Ru => format!("Ошибка уборки: {err}"),
        }
    }

    fn undo_error(&self, err: &str) -> String {
        match self.lang {
            Language::En => format!("Restore failed: {err}"),
            Language::Ru => format!("Ошибка восстановления: {err}"),
        }
    }

    fn archive_create_error(&self, err: &str) -> String {
        match self.lang {
            Language::En => format!("Could not create the archive: {err}"),
            Language::Ru => format!("Не удалось создать архив: {err}"),
        }
    }

    fn archive_opened(&self, path: &Path) -> String {
        match self.lang {
            Language::En => format!("Archive opened: {}", path.display()),
            Language::Ru => format!("Открыт архив: {}", path.display()),
        }
    }

    fn archive_open_error(&self, err: &str) -> String {
        match self.lang {
            Language::En => format!("Could not open the archive: {err}"),
            Language::Ru => format!("Не удалось открыть архив: {err}"),
        }
    }
}


/// Открывает окно настроек. Используется в дочернем процессе (`--settings`),
/// поэтому после сохранения процесс завершается и память полностью освобождается.
pub fn run_settings_window(
    config_path: PathBuf,
    journal_path: PathBuf,
    desktop_dir: PathBuf,
) -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([420.0, 620.0])
            .with_resizable(false),
        centered: true,
        ..Default::default()
    };

    eframe::run_native(
        "Desktop Cleaner",
        options,
        Box::new(move |cc| {
            Ok(Box::new(SettingsApp::new(
                cc,
                config_path,
                journal_path,
                desktop_dir,
            )))
        }),
    )
}

/// Запускает окно настроек отдельным процессом, чтобы оно не занимало память,
/// пока свёрнуто. При сохранении процесс завершается сам.
pub fn spawn_settings_window() -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    Command::new(exe).arg("--settings").spawn().map(|_| ())
}

struct SettingsApp {
    config_path: PathBuf,
    journal_path: PathBuf,
    desktop_dir: PathBuf,
    language: Language,
    enabled: bool,
    launch_at_login: bool,
    mode_index: usize,
    formats_text: String,
    exceptions_text: String,
    prefixes_text: String,
    timer_index: usize,
    minutes_text: String,
    daily_text: String,
    archive_text: String,
    auto_delete_days: Option<u32>,
    by_category: bool,
    status: String,
    status_is_error: bool,
    should_close: bool,
}

impl SettingsApp {
    fn new(
        cc: &eframe::CreationContext<'_>,
        config_path: PathBuf,
        journal_path: PathBuf,
        desktop_dir: PathBuf,
    ) -> Self {
        configure_style(&cc.egui_ctx);
        let config = Config::load_from(&config_path).unwrap_or_default();

        let mode_index = match &config.mode {
            CleanMode::Nothing => 0,
            CleanMode::OnlyScreenshots => 1,
            CleanMode::OnlyFolders => 2,
            CleanMode::SelectedFormats(_) => 3,
        };
        let formats_text = match &config.mode {
            CleanMode::SelectedFormats(formats) => formats.join(", "),
            _ => String::new(),
        };

        let minutes_text = match &config.timer {
            Timer::Minutes(minutes) => minutes.to_string(),
            _ => "15".to_string(),
        };
        let daily_text = match &config.timer {
            Timer::Daily(time) => time.clone(),
            _ => "03:00".to_string(),
        };
        let timer_index = match &config.timer {
            Timer::Immediate => 0,
            Timer::Minutes(_) => 1,
            Timer::Daily(_) => 2,
        };

        Self {
            config_path,
            journal_path,
            desktop_dir,
            language: config.resolved_language(),
            enabled: config.enabled,
            // Показываем реальное состояние системы, а не только запись в конфиге.
            launch_at_login: launch_agent_status().unwrap_or(config.launch_at_login),
            mode_index,
            formats_text,
            exceptions_text: config.exceptions.join("\n"),
            prefixes_text: config.screenshot_prefixes.join("\n"),
            timer_index,
            minutes_text,
            daily_text,
            archive_text: config.archive_dir.to_string_lossy().into_owned(),
            auto_delete_days: config.auto_delete_days,
            by_category: config.sort_mode == SortMode::ByCategory,
            status: String::new(),
            status_is_error: false,
            should_close: false,
        }
    }

    fn build_config(&self) -> Result<Config, String> {
        let t = strings(self.language);

        let mode = match self.mode_index {
            0 => CleanMode::Nothing,
            1 => CleanMode::OnlyScreenshots,
            2 => CleanMode::OnlyFolders,
            _ => CleanMode::SelectedFormats(split_list(&self.formats_text)),
        };

        let timer = match self.timer_index {
            0 => Timer::Immediate,
            1 => {
                let minutes = self
                    .minutes_text
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| t.err_minutes_number.to_string())?;
                if minutes == 0 {
                    return Err(t.err_minutes_positive.to_string());
                }
                Timer::Minutes(minutes)
            }
            _ => {
                let raw = self.daily_text.trim();
                let parsed = chrono::NaiveTime::parse_from_str(raw, "%H:%M")
                    .or_else(|_| chrono::NaiveTime::parse_from_str(raw, "%H:%M:%S"))
                    .map_err(|_| t.err_time_format.to_string())?;
                Timer::Daily(parsed.format("%H:%M").to_string())
            }
        };

        let archive_text = self.archive_text.trim();
        let archive_dir = if archive_text.is_empty() {
            default_archive_dir()
        } else {
            PathBuf::from(expand_tilde(archive_text))
        };

        Ok(Config {
            enabled: self.enabled,
            launch_at_login: self.launch_at_login,
            language: Some(self.language),
            mode,
            exceptions: split_list(&self.exceptions_text),
            screenshot_prefixes: split_list(&self.prefixes_text),
            timer,
            archive_dir,
            auto_delete_days: self.auto_delete_days,
            sort_mode: if self.by_category {
                SortMode::ByCategory
            } else {
                SortMode::ByDate
            },
        })
    }

    fn save(&mut self) {
        let t = strings(self.language);
        match self.build_config() {
            Ok(config) => match config.save_to(&self.config_path) {
                Ok(()) => {
                    self.status = t.status_saved.into();
                    self.status_is_error = false;
                    self.should_close = true;
                }
                Err(err) => {
                    self.status = t.save_error(&err.to_string());
                    self.status_is_error = true;
                }
            },
            Err(message) => {
                self.status = message;
                self.status_is_error = true;
            }
        }
    }

    fn toggle_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
        let t = strings(self.language);

        // Пишем только главный тумблер, сохраняя остальные настройки с диска.
        // На первом запуске файла ещё нет — берём текущую форму.
        let mut config = if self.config_path.exists() {
            Config::load_from(&self.config_path)
                .unwrap_or_else(|_| self.build_config().unwrap_or_default())
        } else {
            self.build_config().unwrap_or_default()
        };
        config.enabled = enabled;

        match config.save_to(&self.config_path) {
            Ok(()) => {
                self.status = if enabled {
                    t.status_auto_on.into()
                } else {
                    t.status_auto_off.into()
                };
                self.status_is_error = false;
            }
            Err(err) => {
                self.status = t.auto_save_error(&err.to_string());
                self.status_is_error = true;
            }
        }
    }

    /// Мгновенно переключает язык интерфейса и сохраняет выбор в config.json.
    fn set_language(&mut self, language: Language) {
        self.language = language;
        let t = strings(language);

        let mut config = if self.config_path.exists() {
            Config::load_from(&self.config_path)
                .unwrap_or_else(|_| self.build_config().unwrap_or_default())
        } else {
            self.build_config().unwrap_or_default()
        };
        config.language = Some(language);

        match config.save_to(&self.config_path) {
            Ok(()) => {
                self.status = t.language_changed.into();
                self.status_is_error = false;
            }
            Err(err) => {
                self.status = t.save_error(&err.to_string());
                self.status_is_error = true;
            }
        }
    }

    fn set_launch_at_login(&mut self, enabled: bool) {
        let t = strings(self.language);

        match sync_launch_agent(enabled) {
            Ok(()) => {
                let actual = launch_agent_status().unwrap_or(enabled);
                self.launch_at_login = actual;

                let mut config = if self.config_path.exists() {
                    Config::load_from(&self.config_path)
                        .unwrap_or_else(|_| self.build_config().unwrap_or_default())
                } else {
                    self.build_config().unwrap_or_default()
                };
                config.launch_at_login = actual;

                if let Err(err) = config.save_to(&self.config_path) {
                    self.status = t.save_error(&err.to_string());
                    self.status_is_error = true;
                    return;
                }

                self.status = match (actual, self.language) {
                    (true, Language::Ru) => "Автозапуск при входе включён.".into(),
                    (true, Language::En) => "Launch at login enabled.".into(),
                    (false, Language::Ru) => "Автозапуск при входе выключен.".into(),
                    (false, Language::En) => "Launch at login disabled.".into(),
                };
                self.status_is_error = false;
            }
            Err(err) => {
                self.launch_at_login = launch_agent_status().unwrap_or(!enabled);
                self.status = match self.language {
                    Language::Ru => format!("Не удалось обновить автозапуск: {err}"),
                    Language::En => format!("Could not update launch at login: {err}"),
                };
                self.status_is_error = true;
            }
        }
    }

    fn clean_now(&mut self) {
        let t = strings(self.language);
        let config = match self.build_config() {
            Ok(config) => config,
            Err(message) => {
                self.status = message;
                self.status_is_error = true;
                return;
            }
        };

        // Сначала фиксируем текущие настройки, затем убираем по ним.
        if let Err(err) = config.save_to(&self.config_path) {
            self.status = t.save_error(&err.to_string());
            self.status_is_error = true;
            return;
        }

        match run_cleanup_once(&self.desktop_dir, &config, &self.journal_path) {
            Ok(0) => {
                self.status = t.status_nothing.into();
                self.status_is_error = false;
            }
            Ok(count) => {
                self.status = t.moved(count, &config.archive_dir);
                self.status_is_error = false;
            }
            Err(err) => {
                self.status = t.cleanup_error(&err.to_string());
                self.status_is_error = true;
            }
        }
    }

    fn undo(&mut self) {
        let t = strings(self.language);
        match undo_last_batch(&self.journal_path) {
            Ok(0) => {
                self.status = t.status_history_empty.into();
                self.status_is_error = false;
            }
            Ok(count) => {
                self.status = t.restored(count);
                self.status_is_error = false;
            }
            Err(err) => {
                self.status = t.undo_error(&err.to_string());
                self.status_is_error = true;
            }
        }
    }

    fn open_archive(&mut self) {
        let t = strings(self.language);
        let archive_dir = self
            .build_config()
            .map(|config| config.archive_dir)
            .unwrap_or_else(|_| Config::load_from(&self.config_path).unwrap_or_default().archive_dir);

        if let Err(err) = fs::create_dir_all(&archive_dir) {
            self.status = t.archive_create_error(&err.to_string());
            self.status_is_error = true;
            return;
        }

        match std::process::Command::new("open").arg(&archive_dir).spawn() {
            Ok(_) => {
                self.status = t.archive_opened(&archive_dir);
                self.status_is_error = false;
            }
            Err(err) => {
                self.status = t.archive_open_error(&err.to_string());
                self.status_is_error = true;
            }
        }
    }

    fn footer(&mut self, ui: &mut Ui) {
        let t = strings(self.language);
        let width = ui.available_width();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            let button_width = ((width - 8.0) / 2.0).max(80.0);

            if ui
                .add_sized([button_width, 34.0], secondary_button(t.undo))
                .clicked()
            {
                self.undo();
            }

            if ui
                .add_sized([button_width, 34.0], secondary_button(t.clean_now))
                .clicked()
            {
                self.clean_now();
            }
        });

        ui.add_space(8.0);

        let save_button = egui::Button::new(
            RichText::new(t.save)
                .size(13.0)
                .strong()
                .color(BG),
        )
        .fill(ACCENT)
        .stroke(Stroke::NONE)
        .rounding(10.0);

        if ui.add_sized([width, 42.0], save_button).clicked() {
            self.save();
        }

        ui.add_space(4.0);
        let status = if self.status.is_empty() {
            t.ready.to_string()
        } else {
            self.status.clone()
        };
        let status_is_error = self.status_is_error;
        let status_has_message = !self.status.is_empty();
        let mut open_clicked = false;

        ui.horizontal(|ui| {
            let label_width = (ui.available_width() - 88.0).max(100.0);
            ui.add_sized(
                [label_width, 22.0],
                egui::Label::new(
                    RichText::new(status).size(10.0).color(if status_is_error {
                        WARN
                    } else if status_has_message {
                        ACCENT
                    } else {
                        MUTED
                    }),
                )
                .truncate(),
            );

            let open_button = egui::Button::new(
                RichText::new(t.open_archive).size(10.0).color(TEXT),
            )
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0_f32, BORDER))
            .rounding(7.0);

            if ui.add_sized([82.0, 22.0], open_button).clicked() {
                open_clicked = true;
            }
        });

        if open_clicked {
            self.open_archive();
        }
    }
}

impl eframe::App for SettingsApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default()
            .frame(
                egui::Frame::none()
                    .fill(BG)
                    .inner_margin(Margin::same(18.0)),
            )
            .show(ctx, |ui| {
                let t = strings(self.language);

                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("Desktop Cleaner")
                                .size(19.0)
                                .strong()
                                .color(TEXT),
                        );
                        ui.label(RichText::new(t.subtitle).size(11.0).color(MUTED));
                    });

                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            let mut language = self.language;
                            if language_switch(ui, &mut language) {
                                self.set_language(language);
                            }
                        },
                    );
                });
                ui.add_space(10.0);

                let mut enabled = self.enabled;
                let mut launch_at_login = self.launch_at_login;
                auto_card(ui, &mut enabled, &mut launch_at_login, t);
                if enabled != self.enabled {
                    self.toggle_enabled(enabled);
                }
                if launch_at_login != self.launch_at_login {
                    self.set_launch_at_login(launch_at_login);
                }

                let footer_height = 150.0;
                let scroll_height = (ui.available_height() - footer_height).max(120.0);

                egui::ScrollArea::vertical()
                    .max_height(scroll_height)
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        card(ui, t.section_clean, |ui| {
                            for (index, label) in t.modes.iter().enumerate() {
                                choice_row(ui, &mut self.mode_index, index, label);
                            }

                            if self.mode_index == 3 {
                                ui.add_space(6.0);
                                ui.label(field_label(t.formats_label));
                                ui.add(
                                    egui::TextEdit::multiline(&mut self.formats_text)
                                        .desired_rows(1)
                                        .desired_width(f32::INFINITY)
                                        .hint_text(t.formats_hint),
                                );
                            }
                        });

                        card(ui, t.section_exceptions, |ui| {
                            ui.label(field_label(t.exceptions_label));
                            ui.add(
                                egui::TextEdit::multiline(&mut self.exceptions_text)
                                    .desired_rows(2)
                                    .desired_width(f32::INFINITY)
                                    .hint_text(t.exceptions_hint),
                            );
                            ui.add_space(6.0);
                            ui.label(field_label(t.prefixes_label));
                            ui.add(
                                egui::TextEdit::multiline(&mut self.prefixes_text)
                                    .desired_rows(2)
                                    .desired_width(f32::INFINITY)
                                    .hint_text(t.prefixes_hint),
                            );
                        });

                        card(ui, t.section_schedule, |ui| {
                            choice_row(
                                ui,
                                &mut self.timer_index,
                                0,
                                t.timer_immediate,
                            );
                            ui.label(
                                RichText::new(t.timer_immediate_hint)
                                    .size(10.0)
                                    .color(MUTED),
                            );
                            ui.add_space(2.0);
                            choice_row(ui, &mut self.timer_index, 1, t.timer_minutes);
                            choice_row(ui, &mut self.timer_index, 2, t.timer_daily);

                            match self.timer_index {
                                1 => {
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(t.interval).size(11.0).color(MUTED),
                                        );
                                        ui.add(
                                            egui::TextEdit::singleline(&mut self.minutes_text)
                                                .desired_width(54.0),
                                        );
                                        ui.label(
                                            RichText::new(t.minutes_word)
                                                .size(11.0)
                                                .color(MUTED),
                                        );
                                    });
                                }
                                2 => {
                                    ui.add_space(6.0);
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(t.time_label)
                                                .size(11.0)
                                                .color(MUTED),
                                        );
                                        ui.add(
                                            egui::TextEdit::singleline(&mut self.daily_text)
                                                .desired_width(64.0)
                                                .hint_text("03:00"),
                                        );
                                        ui.label(
                                            RichText::new("HH:MM").size(11.0).color(MUTED),
                                        );
                                    });
                                }
                                _ => {}
                            }
                        });

                        card(ui, t.section_archive, |ui| {
                            ui.label(field_label(t.archive_folder));
                            ui.horizontal(|ui| {
                                let button_width = 84.0;
                                let edit_width =
                                    (ui.available_width() - button_width - 8.0).max(120.0);
                                ui.add_sized(
                                    [edit_width, 24.0],
                                    egui::TextEdit::singleline(&mut self.archive_text),
                                );
                                if ui
                                    .add_sized(
                                        [button_width, 24.0],
                                        secondary_button(t.choose),
                                    )
                                    .clicked()
                                {
                                    if let Some(folder) =
                                        rfd::FileDialog::new().pick_folder()
                                    {
                                        self.archive_text =
                                            folder.to_string_lossy().into_owned();
                                    }
                                }
                            });

                            ui.add_space(8.0);
                            toggle(ui, &mut self.by_category, t.sort_category);
                            ui.add_space(2.0);
                            ui.label(RichText::new(t.sort_hint).size(10.0).color(MUTED));

                            ui.add_space(10.0);
                            let mut auto_delete = self.auto_delete_days.is_some();
                            toggle(ui, &mut auto_delete, t.auto_delete);

                            if auto_delete != self.auto_delete_days.is_some() {
                                self.auto_delete_days = if auto_delete {
                                    let days = self.auto_delete_days.unwrap_or(7);
                                    Some(if [3, 5, 7, 30].contains(&days) {
                                        days
                                    } else {
                                        7
                                    })
                                } else {
                                    None
                                };
                            }

                            if let Some(days) = self.auto_delete_days {
                                ui.add_space(6.0);
                                ui.label(field_label(t.retention_label));
                                ui.horizontal(|ui| {
                                    for option in [3u32, 5, 7, 30] {
                                        if chip(
                                            ui,
                                            days == option,
                                            &format!("{option} {}", t.days_suffix),
                                        ) {
                                            self.auto_delete_days = Some(option);
                                        }
                                    }
                                });
                                ui.add_space(2.0);
                                ui.label(
                                    RichText::new(t.retention_hint)
                                        .size(10.0)
                                        .color(MUTED),
                                );
                            }
                        });
                    });

                ui.add_space(8.0);
                self.footer(ui);
            });

        if self.should_close {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
    }
}

fn configure_style(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.extreme_bg_color = FIELD;
    visuals.faint_bg_color = CARD;
    visuals.selection.bg_fill = ACCENT;
    visuals.selection.stroke = Stroke::new(1.0_f32, BG);
    visuals.override_text_color = Some(TEXT);

    visuals.widgets.noninteractive.bg_fill = CARD;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, MUTED);
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);

    visuals.widgets.inactive.bg_fill = Color32::from_rgb(0x26, 0x29, 0x2F);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(0x26, 0x29, 0x2F);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);

    visuals.widgets.hovered.bg_fill = BORDER;
    visuals.widgets.hovered.weak_bg_fill = BORDER;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, TEXT);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, Color32::from_rgb(0x3A, 0x3E, 0x46));

    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.weak_bg_fill = ACCENT;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, BG);

    ctx.style_mut(|style| {
        style.visuals = visuals;
        style.spacing.item_spacing = vec2(8.0, 8.0);
        style.spacing.button_padding = vec2(12.0, 8.0);
        style.visuals.window_rounding = Rounding::same(12.0);
    });
}

fn language_switch(ui: &mut Ui, language: &mut Language) -> bool {
    let mut changed = false;

    egui::Frame::none()
        .fill(FIELD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(Rounding::same(8.0))
        .inner_margin(Margin::same(3.0))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (value, label) in [(Language::En, "EN"), (Language::Ru, "RU")] {
                    let selected = *language == value;
                    let button = egui::Button::new(
                        RichText::new(label)
                            .size(10.0)
                            .strong()
                            .color(if selected { BG } else { MUTED }),
                    )
                    .fill(if selected {
                        ACCENT
                    } else {
                        Color32::TRANSPARENT
                    })
                    .stroke(Stroke::NONE)
                    .rounding(Rounding::same(6.0));

                    if ui.add_sized([30.0, 20.0], button).clicked() && !selected {
                        *language = value;
                        changed = true;
                    }
                }
            });
        });

    changed
}

fn auto_card(ui: &mut Ui, enabled: &mut bool, launch_at_login: &mut bool, t: &Strings) {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(12.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(t.auto_title)
                            .size(10.0)
                            .strong()
                            .color(MUTED),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(if *enabled { t.auto_on } else { t.auto_off })
                            .size(13.0)
                            .color(if *enabled { TEXT } else { MUTED }),
                    );
                });

                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| big_toggle(ui, enabled),
                );
            });

            ui.add_space(10.0);
            ui.add(egui::Separator::default().spacing(0.0));
            ui.add_space(8.0);
            toggle(ui, launch_at_login, t.launch_at_login);
        });
    ui.add_space(8.0);
}

fn big_toggle(ui: &mut Ui, value: &mut bool) {
    let (rect, response) = ui.allocate_exact_size(vec2(54.0, 30.0), Sense::click());
    if response.clicked() {
        *value = !*value;
    }

    let t = ui.ctx().animate_bool(response.id, *value);
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        15.0,
        if *value {
            ACCENT
        } else {
            Color32::from_rgb(0x3A, 0x3D, 0x44)
        },
    );
    painter.rect_stroke(
        rect,
        15.0,
        Stroke::new(
            1.0_f32,
            if *value { ACCENT } else { BORDER },
        ),
    );

    let knob_x = rect.left() + 15.0 + t * (rect.width() - 30.0);
    painter.circle_filled(
        pos2(knob_x, rect.center().y),
        11.0,
        if *value {
            BG
        } else {
            Color32::from_rgb(0xC9, 0xCD, 0xD3)
        },
    );
}

fn card(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
    egui::Frame::none()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(Rounding::same(12.0))
        .inner_margin(Margin::same(14.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).size(10.0).strong().color(MUTED));
            ui.add_space(6.0);
            add(ui);
        });
    ui.add_space(8.0);
}

fn field_label(text: &str) -> RichText {
    RichText::new(text).size(10.0).color(MUTED)
}

fn choice_row(ui: &mut Ui, selected: &mut usize, index: usize, label: &str) {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(vec2(width, 30.0), Sense::click());

    if response.clicked() {
        *selected = index;
    }

    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect, 8.0, Color32::from_rgb(0x26, 0x29, 0x2F));
    }

    let center = pos2(rect.left() + 13.0, rect.center().y);
    let is_selected = *selected == index;
    painter.circle_stroke(
        center,
        6.0,
        Stroke::new(1.5_f32, Color32::from_rgb(0x4A, 0x4E, 0x57)),
    );
    if is_selected {
        painter.circle_filled(center, 3.5, ACCENT);
    }

    painter.text(
        pos2(rect.left() + 28.0, rect.center().y),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(12.5),
        if is_selected { TEXT } else { Color32::from_rgb(0xC4, 0xC8, 0xCF) },
    );
}

fn toggle(ui: &mut Ui, value: &mut bool, label: &str) {
    ui.horizontal(|ui| {
        let (rect, response) = ui.allocate_exact_size(vec2(42.0, 24.0), Sense::click());
        if response.clicked() {
            *value = !*value;
        }

        let t = ui.ctx().animate_bool(response.id, *value);
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            rect.height() / 2.0,
            if *value {
                ACCENT
            } else {
                Color32::from_rgb(0x33, 0x36, 0x3D)
            },
        );
        let knob_x = rect.left() + 12.0 + t * (rect.width() - 24.0);
        painter.circle_filled(
            pos2(knob_x, rect.center().y),
            9.0,
            Color32::from_rgb(0x16, 0x17, 0x1A),
        );

        ui.add_space(6.0);
        ui.label(RichText::new(label).size(12.0).color(TEXT));
    });
}

fn chip(ui: &mut Ui, selected: bool, label: &str) -> bool {
    let button = egui::Button::new(
        RichText::new(label)
            .size(10.5)
            .color(if selected { BG } else { TEXT }),
    )
    .fill(if selected {
        ACCENT
    } else {
        Color32::from_rgb(0x26, 0x29, 0x2F)
    })
    .stroke(Stroke::new(
        1.0_f32,
        if selected { ACCENT } else { BORDER },
    ))
    .rounding(8.0);

    ui.add_sized([52.0, 24.0], button).clicked()
}

fn secondary_button(text: &str) -> egui::Button<'static> {
    egui::Button::new(RichText::new(text).size(11.0).color(TEXT))
        .fill(Color32::from_rgb(0x26, 0x29, 0x2F))
        .stroke(Stroke::new(1.0_f32, BORDER))
        .rounding(9.0)
}

fn split_list(text: &str) -> Vec<String> {
    text.split(['\n', ','])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}

fn objects_word(count: usize) -> &'static str {
    let last_two = count % 100;
    let last = count % 10;

    if (11..=14).contains(&last_two) {
        "объектов"
    } else {
        match last {
            1 => "объект",
            2..=4 => "объекта",
            _ => "объектов",
        }
    }
}

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest).to_string_lossy().into_owned();
        }
    }
    path.to_string()
}

// ---------------------------------------------------------------------------
// Менюбар
// ---------------------------------------------------------------------------

enum TrayUserEvent {
    Menu(MenuEvent),
}

struct TrayApp {
    config_path: PathBuf,
    journal_path: PathBuf,
    desktop_dir: PathBuf,
    stop_signal: Arc<AtomicBool>,
    tray: Option<TrayIcon>,
    enabled_item: Option<MenuItem>,
    launch_item: Option<CheckMenuItem>,
    settings_item: Option<MenuItem>,
    cleanup_item: Option<MenuItem>,
    undo_item: Option<MenuItem>,
    quit_item: Option<MenuItem>,
    enabled_id: Option<MenuId>,
    launch_id: Option<MenuId>,
    settings_id: Option<MenuId>,
    cleanup_id: Option<MenuId>,
    undo_id: Option<MenuId>,
    quit_id: Option<MenuId>,
    last_enabled_sync: Instant,
    last_launch_sync: Instant,
    exit: bool,
}

/// Запускает менюбар на главном потоке. Блокируется до выбора «Выход».
pub fn run_tray(
    config_path: PathBuf,
    journal_path: PathBuf,
    desktop_dir: PathBuf,
    stop_signal: Arc<AtomicBool>,
) -> Result<(), Box<dyn Error>> {
    let event_loop = EventLoop::<TrayUserEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |event| {
        let _ = proxy.send_event(TrayUserEvent::Menu(event));
    }));

    let mut app = TrayApp {
        config_path,
        journal_path,
        desktop_dir,
        stop_signal,
        tray: None,
        enabled_item: None,
        launch_item: None,
        settings_item: None,
        cleanup_item: None,
        undo_item: None,
        quit_item: None,
        enabled_id: None,
        launch_id: None,
        settings_id: None,
        cleanup_id: None,
        undo_id: None,
        quit_id: None,
        last_enabled_sync: Instant::now(),
        last_launch_sync: Instant::now(),
        exit: false,
    };

    event_loop.run_app(&mut app)?;
    Ok(())
}

impl TrayApp {
    fn ensure_tray(&mut self) {
        if self.tray.is_some() {
            return;
        }

        match build_tray(&Config::load_from(&self.config_path).unwrap_or_default()) {
            Ok(handles) => {
                self.tray = Some(handles.tray);
                self.enabled_id = Some(handles.enabled_id);
                self.enabled_item = Some(handles.enabled_item);
                self.launch_id = Some(handles.launch_id);
                self.launch_item = Some(handles.launch_item);
                self.settings_item = Some(handles.settings_item);
                self.cleanup_item = Some(handles.cleanup_item);
                self.undo_item = Some(handles.undo_item);
                self.quit_item = Some(handles.quit_item);
                self.settings_id = Some(handles.settings_id);
                self.cleanup_id = Some(handles.cleanup_id);
                self.undo_id = Some(handles.undo_id);
                self.quit_id = Some(handles.quit_id);
            }
            Err(err) => {
                eprintln!("desktop_cleaner: не удалось создать иконку менюбара: {err}");
                self.exit = true;
            }
        }
    }

    /// Переключает автозапуск из меню трея.
    fn toggle_launch_at_login(&mut self) {
        if !self.config_path.exists() {
            if let Err(err) = spawn_settings_window() {
                eprintln!("desktop_cleaner: не удалось открыть настройки: {err}");
            }
            return;
        }

        let enabled = self
            .launch_item
            .as_ref()
            .map(|item| item.is_checked())
            .unwrap_or(false);

        match sync_launch_agent(enabled) {
            Ok(()) => {
                let actual = launch_agent_status().unwrap_or(enabled);
                let mut config = match Config::load_from(&self.config_path) {
                    Ok(config) => config,
                    Err(err) => {
                        eprintln!("desktop_cleaner: не удалось прочитать настройки: {err}");
                        self.sync_launch_check();
                        return;
                    }
                };
                config.launch_at_login = actual;
                if let Err(err) = config.save_to(&self.config_path) {
                    eprintln!("desktop_cleaner: не удалось сохранить настройки: {err}");
                }
            }
            Err(err) => {
                eprintln!("desktop_cleaner: не удалось изменить автозапуск: {err}");
            }
        }

        self.sync_launch_check();
        self.last_launch_sync = Instant::now();
    }

    /// Приводит галочку в меню к реальному состоянию launchd.
    fn sync_launch_check(&self) {
        let Some(item) = &self.launch_item else {
            return;
        };
        let Some(actual) = launch_agent_status() else {
            return;
        };
        if item.is_checked() != actual {
            item.set_checked(actual);
        }
    }

    /// Переключает автоочистку из трея и сразу пишет состояние в config.json.
    fn toggle_enabled(&mut self) {
        if !self.config_path.exists() {
            // Настроек ещё нет — безопаснее открыть окно, чем писать дефолты.
            if let Err(err) = spawn_settings_window() {
                eprintln!("desktop_cleaner: не удалось открыть настройки: {err}");
            }
            return;
        }

        let mut config = match Config::load_from(&self.config_path) {
            Ok(config) => config,
            Err(err) => {
                eprintln!("desktop_cleaner: не удалось прочитать настройки: {err}");
                return;
            }
        };

        config.enabled = !config.enabled;
        if let Err(err) = config.save_to(&self.config_path) {
            eprintln!("desktop_cleaner: не удалось сохранить настройки: {err}");
            return;
        }

        self.sync_labels();
    }

    /// Подхватывает состояние и язык из config.json (их могли изменить в окне).
    fn sync_labels(&self) {
        let config = Config::load_from(&self.config_path).unwrap_or_default();
        let t = strings(config.resolved_language());

        set_item_text(
            &self.enabled_item,
            if config.enabled { t.pause } else { t.enable_auto },
        );
        if let Some(item) = &self.launch_item {
            if item.text() != t.launch_at_login {
                item.set_text(t.launch_at_login);
            }
        }
        set_item_text(&self.settings_item, t.settings);
        set_item_text(&self.cleanup_item, t.clean_now);
        set_item_text(&self.undo_item, t.undo);
        set_item_text(&self.quit_item, t.quit);
    }

    fn handle_menu(&mut self, id: &MenuId) {
        if self.enabled_id.as_ref() == Some(id) {
            self.toggle_enabled();
        } else if self.launch_id.as_ref() == Some(id) {
            self.toggle_launch_at_login();
        } else if self.settings_id.as_ref() == Some(id) {
            if let Err(err) = spawn_settings_window() {
                eprintln!("desktop_cleaner: не удалось открыть настройки: {err}");
            }
        } else if self.cleanup_id.as_ref() == Some(id) {
            let config_path = self.config_path.clone();
            let journal_path = self.journal_path.clone();
            let desktop_dir = self.desktop_dir.clone();
            std::thread::spawn(move || {
                let config = Config::load_from(&config_path).unwrap_or_default();
                match run_cleanup_once(&desktop_dir, &config, &journal_path) {
                    Ok(count) => eprintln!("desktop_cleaner: убрано объектов: {count}"),
                    Err(err) => eprintln!("desktop_cleaner: ошибка уборки: {err}"),
                }
            });
        } else if self.undo_id.as_ref() == Some(id) {
            let journal_path = self.journal_path.clone();
            std::thread::spawn(move || match undo_last_batch(&journal_path) {
                Ok(count) => eprintln!("desktop_cleaner: восстановлено объектов: {count}"),
                Err(err) => eprintln!("desktop_cleaner: ошибка восстановления: {err}"),
            });
        } else if self.quit_id.as_ref() == Some(id) {
            self.exit = true;
        }
    }
}

impl ApplicationHandler<TrayUserEvent> for TrayApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        _event: winit::event::WindowEvent,
    ) {
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if cause == StartCause::Init {
            self.ensure_tray();
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(100),
            ));
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: TrayUserEvent) {
        match event {
            TrayUserEvent::Menu(event) => self.handle_menu(&event.id),
        }

        if self.exit {
            event_loop.exit();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.stop_signal.load(Ordering::Relaxed) {
            self.exit = true;
        }

        if self.exit {
            event_loop.exit();
            return;
        }

        if self.last_enabled_sync.elapsed() >= Duration::from_secs(2) {
            self.last_enabled_sync = Instant::now();
            self.sync_labels();
        }

        if self.last_launch_sync.elapsed() >= Duration::from_secs(10) {
            self.last_launch_sync = Instant::now();
            self.sync_launch_check();
        }

        event_loop.set_control_flow(ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(500),
        ));
    }
}

struct TrayHandles {
    tray: TrayIcon,
    enabled_item: MenuItem,
    launch_item: CheckMenuItem,
    settings_item: MenuItem,
    cleanup_item: MenuItem,
    undo_item: MenuItem,
    quit_item: MenuItem,
    enabled_id: MenuId,
    launch_id: MenuId,
    settings_id: MenuId,
    cleanup_id: MenuId,
    undo_id: MenuId,
    quit_id: MenuId,
}

fn set_item_text(item: &Option<MenuItem>, text: &str) {
    if let Some(item) = item {
        if item.text() != text {
            item.set_text(text);
        }
    }
}

fn build_tray(config: &Config) -> Result<TrayHandles, Box<dyn Error>> {
    let t = strings(config.resolved_language());
    let menu = Menu::new();
    let auto = MenuItem::new(
        if config.enabled { t.pause } else { t.enable_auto },
        true,
        None,
    );
    let launch_enabled = launch_agent_status().unwrap_or(config.launch_at_login);
    let launch = CheckMenuItem::new(t.launch_at_login, true, launch_enabled, None);
    let settings = MenuItem::new(t.settings, true, None);
    let cleanup = MenuItem::new(t.clean_now, true, None);
    let undo = MenuItem::new(t.undo, true, None);
    let quit = MenuItem::new(t.quit, true, None);

    menu.append(&auto)?;
    menu.append(&launch)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&settings)?;
    menu.append(&cleanup)?;
    menu.append(&undo)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit)?;

    let icon = Icon::from_rgba(template_icon_rgba(), 32, 32)?;
    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Desktop Cleaner")
        .with_icon(icon)
        .with_icon_as_template(true)
        .with_menu_on_left_click(true)
        .build()?;

    Ok(TrayHandles {
        tray,
        enabled_id: auto.id().clone(),
        enabled_item: auto,
        launch_id: launch.id().clone(),
        launch_item: launch,
        settings_item: settings.clone(),
        cleanup_item: cleanup.clone(),
        undo_item: undo.clone(),
        quit_item: quit.clone(),
        settings_id: settings.id().clone(),
        cleanup_id: cleanup.id().clone(),
        undo_id: undo.id().clone(),
        quit_id: quit.id().clone(),
    })
}

/// Рисует минималистичную white-on-alpha иконку 32×32: рамка «архива» и стрелка.
fn template_icon_rgba() -> Vec<u8> {
    const SIZE: usize = 32;
    const SAMPLES: usize = 4;
    let mut rgba = vec![0u8; SIZE * SIZE * 4];

    for y in 0..SIZE {
        for x in 0..SIZE {
            let mut hits = 0;
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let px = x as f32 + (sx as f32 + 0.5) / SAMPLES as f32;
                    let py = y as f32 + (sy as f32 + 0.5) / SAMPLES as f32;
                    if in_icon_shape(px, py) {
                        hits += 1;
                    }
                }
            }

            let index = (y * SIZE + x) * 4;
            rgba[index] = 255;
            rgba[index + 1] = 255;
            rgba[index + 2] = 255;
            rgba[index + 3] = (hits * 255 / (SAMPLES * SAMPLES)) as u8;
        }
    }

    rgba
}

fn in_icon_shape(x: f32, y: f32) -> bool {
    let outer = inside_rounded_rect(x, y, 4.0, 6.0, 28.0, 26.0, 5.0);
    let inner = inside_rounded_rect(x, y, 6.5, 8.5, 25.5, 23.5, 3.2);
    let frame = outer && !inner;

    // Стрелка вниз внутри рамки.
    let shaft = (14.5..=17.5).contains(&x) && (10.0..=18.0).contains(&y);
    let head = (17.0..=22.0).contains(&y) && (x - 16.0).abs() <= (22.0 - y) * 0.9;

    frame || shaft || head
}

fn inside_rounded_rect(
    x: f32,
    y: f32,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    radius: f32,
) -> bool {
    let cx = x.clamp(left + radius, right - radius);
    let cy = y.clamp(top + radius, bottom - radius);
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= radius * radius
}
