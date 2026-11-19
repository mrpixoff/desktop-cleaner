//! Журнал перемещений и откат последней пачки очистки.

use crate::archive::{move_path, unique_destination};
use chrono::Local;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Имя файла журнала по умолчанию.
pub const JOURNAL_FILE: &str = "journal.json";

/// Сколько последних пачек хранится в журнале.
pub const MAX_BATCHES: usize = 50;

/// Один перемещённый объект: откуда и куда.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MovedItem {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// Пачка перемещений, выполненная за один запуск очистки.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CleanupBatch {
    /// Момент создания пачки в формате RFC 3339.
    pub timestamp: String,
    pub items: Vec<MovedItem>,
}

/// История перемещений.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    pub batches: Vec<CleanupBatch>,
}

impl Journal {
    /// Загрузить журнал из `journal.json`.
    ///
    /// Если файла ещё нет, возвращается пустой журнал.
    pub fn load() -> io::Result<Self> {
        Self::load_from(JOURNAL_FILE)
    }

    /// Сохранить журнал в `journal.json`.
    pub fn save(&self) -> io::Result<()> {
        self.save_to(JOURNAL_FILE)
    }

    /// Загрузить журнал из произвольного пути.
    pub fn load_from(path: impl AsRef<Path>) -> io::Result<Self> {
        match fs::read_to_string(path) {
            Ok(contents) => serde_json::from_str(&contents).map_err(|err| {
                io::Error::new(io::ErrorKind::InvalidData, err)
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(err),
        }
    }

    /// Сохранить журнал в произвольный путь.
    pub fn save_to(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let json = serde_json::to_string_pretty(self).map_err(|err| {
            io::Error::new(io::ErrorKind::InvalidData, err)
        })?;
        if let Some(parent) = path.as_ref().parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, json)
    }

    /// Добавить новую пачку с текущей отметкой времени.
    ///
    /// Хранятся только последние [`MAX_BATCHES`] пачек, более старые удаляются.
    pub fn record_batch(&mut self, items: Vec<MovedItem>) {
        self.batches.push(CleanupBatch {
            timestamp: Local::now().to_rfc3339(),
            items,
        });

        if self.batches.len() > MAX_BATCHES {
            let excess = self.batches.len() - MAX_BATCHES;
            self.batches.drain(..excess);
        }
    }
}

/// Откатывает последнюю пачку из журнала: возвращает объекты из `to` в `from`.
///
/// Пачка удаляется из журнала. Если на исходном месте уже есть объект
/// с таким же именем, восстановленный получает суффикс ` (2)`, ` (3)` и т.д.
///
/// Возвращает количество успешно восстановленных элементов. Отсутствующие
/// в архиве объекты пропускаются, ошибки отдельных переносов не прерывают
/// восстановление остальных.
pub fn undo_last_batch(journal_path: &Path) -> io::Result<usize> {
    let mut journal = Journal::load_from(journal_path)?;
    let Some(batch) = journal.batches.pop() else {
        return Ok(0);
    };

    let mut restored = 0usize;
    let mut first_error: Option<io::Error> = None;

    for item in batch.items.iter().rev() {
        if !item.to.exists() {
            continue;
        }

        let result = restore_item(item);
        match result {
            Ok(()) => restored += 1,
            Err(err) => {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
    }

    journal.save_to(journal_path)?;

    if restored == 0 {
        if let Some(err) = first_error {
            return Err(err);
        }
    }

    Ok(restored)
}

fn restore_item(item: &MovedItem) -> io::Result<()> {
    let parent = item
        .from
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)?;

    let name = item.from.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("путь не содержит имени объекта: {}", item.from.display()),
        )
    })?;

    let destination = if item.from.exists() {
        unique_destination(parent, name, item.to.is_dir())
    } else {
        item.from.clone()
    };

    move_path(&item.to, &destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parents");
        }
        fs::write(path, contents).expect("write file");
    }

    fn moved(from: &Path, to: &Path) -> MovedItem {
        MovedItem {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        }
    }

    #[test]
    fn record_batch_keeps_only_last_fifty_batches() {
        let mut journal = Journal::default();

        for index in 0..60 {
            journal.record_batch(vec![MovedItem {
                from: PathBuf::from(format!("/desktop/file-{index}.txt")),
                to: PathBuf::from(format!("/archive/file-{index}.txt")),
            }]);
        }

        assert_eq!(journal.batches.len(), MAX_BATCHES);
        assert_eq!(
            journal.batches.first().unwrap().items[0].from,
            PathBuf::from("/desktop/file-10.txt")
        );
        assert_eq!(
            journal.batches.last().unwrap().items[0].from,
            PathBuf::from("/desktop/file-59.txt")
        );
        assert!(!journal.batches.last().unwrap().timestamp.is_empty());
    }

    #[test]
    fn undo_restores_files_and_folders() {
        let root = tempfile::tempdir().expect("temp dir");
        let desktop = root.path().join("Desktop");
        let archive = root.path().join("Archive");
        let journal_path = root.path().join("journal.json");
        fs::create_dir(&desktop).expect("desktop");

        let file = archive.join("note.txt");
        let folder = archive.join("Project");
        write(&file, "hello");
        write(&folder.join("nested").join("logo.png"), "png");

        let mut journal = Journal::default();
        journal.record_batch(vec![
            moved(&desktop.join("note.txt"), &file),
            moved(&desktop.join("Project"), &folder),
        ]);
        journal.save_to(&journal_path).expect("save journal");

        let restored = undo_last_batch(&journal_path).expect("undo");

        assert_eq!(restored, 2);
        assert_eq!(
            fs::read_to_string(desktop.join("note.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            fs::read_to_string(desktop.join("Project").join("nested").join("logo.png")).unwrap(),
            "png"
        );
        assert!(!file.exists());
        assert!(!folder.exists());

        let reloaded = Journal::load_from(&journal_path).expect("load journal");
        assert!(reloaded.batches.is_empty());
    }

    #[test]
    fn undo_does_not_overwrite_existing_file() {
        let root = tempfile::tempdir().expect("temp dir");
        let desktop = root.path().join("Desktop");
        let archive = root.path().join("Archive");
        let journal_path = root.path().join("journal.json");

        let archived = archive.join("note.txt");
        let current = desktop.join("note.txt");
        write(&archived, "old");
        write(&current, "new");

        let mut journal = Journal::default();
        journal.record_batch(vec![moved(&current, &archived)]);
        journal.save_to(&journal_path).expect("save journal");

        let restored = undo_last_batch(&journal_path).expect("undo");

        assert_eq!(restored, 1);
        assert_eq!(
            fs::read_to_string(&current).unwrap(),
            "new",
            "новый файл на столе не должен перезаписываться"
        );
        assert_eq!(
            fs::read_to_string(desktop.join("note (2).txt")).unwrap(),
            "old",
            "восстановленный файл должен получить суффикс"
        );
        assert!(!archived.exists());
    }

    #[test]
    fn undo_without_journal_returns_zero() {
        let root = tempfile::tempdir().expect("temp dir");
        let path = root.path().join("missing-journal.json");

        assert_eq!(undo_last_batch(&path).expect("undo"), 0);
        assert!(!path.exists());
    }
}
