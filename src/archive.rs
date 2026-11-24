//! Перенос файлов и папок в архив с сортировкой по дате или категории.

use crate::config::{Config, SortMode};
use chrono::Local;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "webp", "heic", "gif"];
const DOCUMENT_EXTENSIONS: &[&str] = &["pdf", "doc", "docx", "txt", "md", "xls", "xlsx"];
const ARCHIVE_EXTENSIONS: &[&str] = &["zip", "rar", "7z", "tar", "gz"];
const CATEGORY_DIRS: &[&str] = &["Images", "Documents", "Archives", "Other"];

/// Перемещает `src` в архив, согласно настройкам [`Config`].
///
/// * [`SortMode::ByDate`] — в `<archive_dir>/YYYY-MM-DD/`;
/// * [`SortMode::ByCategory`] — в `<archive_dir>/Images|Documents|Archives|Other/`.
///
/// Папки переносятся целиком. Если объект с таким именем уже есть,
/// к имени добавляется суффикс ` (2)`, ` (3)` и т.д.
/// Возвращает итоговый путь объекта в архиве.
pub fn move_to_archive(src: &Path, config: &Config) -> io::Result<PathBuf> {
    if !src.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("путь не существует: {}", src.display()),
        ));
    }

    let file_name = src.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("путь не содержит имени объекта: {}", src.display()),
        )
    })?;

    let destination_dir = match config.sort_mode {
        SortMode::ByDate => config
            .archive_dir
            .join(Local::now().format("%Y-%m-%d").to_string()),
        SortMode::ByCategory => config.archive_dir.join(category_for(src)),
    };
    fs::create_dir_all(&destination_dir)?;

    // Защита от переноса папки в саму себя (например, архив выбран внутри
    // переносимой папки).
    if src.is_dir() {
        if let (Ok(src_canon), Ok(dest_canon)) =
            (fs::canonicalize(src), fs::canonicalize(&destination_dir))
        {
            if dest_canon.starts_with(&src_canon) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("нельзя переместить папку внутрь самой себя: {}", src.display()),
                ));
            }
        }
    }

    let destination = unique_destination(&destination_dir, file_name, src.is_dir());
    move_path(src, &destination)?;

    Ok(destination)
}

/// Отправляет в системную Корзину объекты архива старше `days` суток.
///
/// Проверяются файлы и папки непосредственно в `archive_dir`, а также
/// содержимое подпапок дат (`YYYY-MM-DD`) и категорий
/// (`Images`, `Documents`, `Archives`, `Other`). Опустевшие подпапки дат
/// удаляются. Возвращает количество объектов, отправленных в Корзину.
pub fn cleanup_expired_archive(archive_dir: &Path, days: u32) -> io::Result<usize> {
    cleanup_expired_archive_with(archive_dir, days, |path| {
        trash::delete(path).map_err(|err| io::Error::other(err.to_string()))
    })
}

fn cleanup_expired_archive_with<F>(
    archive_dir: &Path,
    days: u32,
    mut delete: F,
) -> io::Result<usize>
where
    F: FnMut(&Path) -> io::Result<()>,
{
    if !archive_dir.exists() {
        return Ok(0);
    }

    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(
            u64::from(days).saturating_mul(86_400),
        ))
        .unwrap_or(SystemTime::UNIX_EPOCH);

    let mut removed = 0usize;

    for entry in fs::read_dir(archive_dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();

        if path.is_dir() && (is_date_dir(&name) || is_category_dir(&name)) {
            for child in fs::read_dir(&path)? {
                let child = child?;
                let child_path = child.path();
                if is_expired(&child_path, cutoff)? {
                    delete(&child_path)?;
                    removed += 1;
                }
            }

            // Пустые папки дат убираем, чтобы архив не разрастался.
            if is_date_dir(&name) && is_dir_empty(&path)? {
                fs::remove_dir(&path)?;
            }
        } else if is_expired(&path, cutoff)? {
            delete(&path)?;
            removed += 1;
        }
    }

    Ok(removed)
}

fn is_date_dir(name: &str) -> bool {
    chrono::NaiveDate::parse_from_str(name, "%Y-%m-%d").is_ok()
}

fn is_category_dir(name: &str) -> bool {
    CATEGORY_DIRS.contains(&name)
}

fn is_expired(path: &Path, cutoff: SystemTime) -> io::Result<bool> {
    // `symlink_metadata` не идёт по ссылке: битая символическая ссылка
    // в архиве не должна ломать всю очистку.
    let modified = fs::symlink_metadata(path)?.modified()?;
    Ok(modified < cutoff)
}

fn is_dir_empty(path: &Path) -> io::Result<bool> {
    Ok(fs::read_dir(path)?.next().is_none())
}

/// Категория архива для объекта: папки всегда попадают в `Other`.
fn category_for(path: &Path) -> &'static str {
    if path.is_dir() {
        return "Other";
    }

    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase());

    match extension.as_deref() {
        Some(ext) if IMAGE_EXTENSIONS.contains(&ext) => "Images",
        Some(ext) if DOCUMENT_EXTENSIONS.contains(&ext) => "Documents",
        Some(ext) if ARCHIVE_EXTENSIONS.contains(&ext) => "Archives",
        _ => "Other",
    }
}

/// Подбирает свободное имя в каталоге, добавляя ` (2)`, ` (3)` и т.д.
pub(crate) fn unique_destination(directory: &Path, file_name: &OsStr, is_dir: bool) -> PathBuf {
    let candidate = directory.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let name_path = Path::new(file_name);
    let stem = name_path.to_string_lossy();
    let stem = match name_path.extension() {
        Some(_) if !is_dir => name_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| stem.into_owned()),
        _ => stem.into_owned(),
    };
    let extension = if is_dir {
        None
    } else {
        name_path
            .extension()
            .map(|ext| format!(".{}", ext.to_string_lossy()))
    };

    let mut counter = 2u64;
    loop {
        let name = match &extension {
            Some(ext) => format!("{stem} ({counter}){ext}"),
            None => format!("{stem} ({counter})"),
        };
        let candidate = directory.join(name);
        if !candidate.exists() {
            return candidate;
        }
        counter += 1;
    }
}

/// Переносит объект, откатываясь на копирование при переносе между дисками.
pub(crate) fn move_path(src: &Path, destination: &Path) -> io::Result<()> {
    match fs::rename(src, destination) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::CrossesDevices => {
            if let Err(copy_err) = copy_recursive(src, destination) {
                let _ = remove_path(destination);
                return Err(copy_err);
            }
            remove_path(src)
        }
        Err(err) => Err(err),
    }
}

fn copy_recursive(src: &Path, destination: &Path) -> io::Result<()> {
    if src.is_dir() {
        fs::create_dir_all(destination)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &destination.join(entry.file_name()))?;
        }
        Ok(())
    } else {
        copy_file_verified(src, destination)
    }
}

fn copy_file_verified(src: &Path, destination: &Path) -> io::Result<()> {
    let src_size = fs::metadata(src)?.len();
    let copied = match fs::copy(src, destination) {
        Ok(copied) => copied,
        Err(err) => {
            let _ = fs::remove_file(destination);
            return Err(err);
        }
    };

    let dst_size = match fs::metadata(destination) {
        Ok(metadata) => metadata.len(),
        Err(err) => {
            let _ = fs::remove_file(destination);
            return Err(err);
        }
    };

    if copied != src_size || dst_size != src_size {
        let _ = fs::remove_file(destination);
        return Err(io::Error::other(format!(
            "размер копии не совпадает с оригиналом: {} (src={src_size}, copied={copied}, dst={dst_size})",
            src.display()
        )));
    }

    Ok(())
}

fn remove_path(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{CleanMode, Config};

    fn config(archive_dir: PathBuf, sort_mode: SortMode) -> Config {
        Config {
            archive_dir,
            sort_mode,
            mode: CleanMode::Nothing,
            ..Config::default()
        }
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parents");
        }
        fs::write(path, contents).expect("write file");
    }

    #[test]
    fn by_date_moves_file_into_dated_folder() {
        let root = tempfile::tempdir().expect("temp dir");
        let archive_dir = root.path().join("archive");
        let source = root.path().join("note.txt");
        write(&source, "hello");

        let destination =
            move_to_archive(&source, &config(archive_dir.clone(), SortMode::ByDate))
                .expect("move file");

        assert!(!source.exists(), "исходный файл должен исчезнуть");
        assert_eq!(fs::read_to_string(&destination).unwrap(), "hello");

        let day_dir = destination.parent().expect("day dir");
        assert_eq!(day_dir.parent(), Some(archive_dir.as_path()));
        let day = day_dir.file_name().unwrap().to_string_lossy().into_owned();
        chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d")
            .expect("каталог должен называться по дате YYYY-MM-DD");
    }

    #[test]
    fn by_category_splits_files_and_folders() {
        let root = tempfile::tempdir().expect("temp dir");
        let archive_dir = root.path().join("archive");
        let cfg = config(archive_dir.clone(), SortMode::ByCategory);

        let image = root.path().join("photo.PNG");
        let document = root.path().join("report.pdf");
        let bundle = root.path().join("bundle.zip");
        let other = root.path().join("data.bin");
        let folder = root.path().join("Projects");
        write(&image, "img");
        write(&document, "doc");
        write(&bundle, "zip");
        write(&other, "bin");
        fs::create_dir(&folder).unwrap();

        let expected = [
            (&image, "Images"),
            (&document, "Documents"),
            (&bundle, "Archives"),
            (&other, "Other"),
            (&folder, "Other"),
        ];

        for (source, category) in expected {
            let destination = move_to_archive(source, &cfg).expect("move");
            assert!(!source.exists());
            assert_eq!(
                destination
                    .parent()
                    .unwrap()
                    .file_name()
                    .unwrap()
                    .to_str(),
                Some(category)
            );
            assert_eq!(destination.parent().unwrap().parent(), Some(archive_dir.as_path()));
        }
    }

    #[test]
    fn name_collisions_get_numeric_suffix() {
        let root = tempfile::tempdir().expect("temp dir");
        let archive_dir = root.path().join("archive");
        let cfg = config(archive_dir.clone(), SortMode::ByCategory);

        let first = root.path().join("a").join("report.pdf");
        let second = root.path().join("b").join("report.pdf");
        let third = root.path().join("c").join("report.pdf");
        write(&first, "1");
        write(&second, "2");
        write(&third, "3");

        let first_dest = move_to_archive(&first, &cfg).unwrap();
        let second_dest = move_to_archive(&second, &cfg).unwrap();
        let third_dest = move_to_archive(&third, &cfg).unwrap();

        assert_eq!(first_dest, archive_dir.join("Documents").join("report.pdf"));
        assert_eq!(
            second_dest,
            archive_dir.join("Documents").join("report (2).pdf")
        );
        assert_eq!(
            third_dest,
            archive_dir.join("Documents").join("report (3).pdf")
        );
        assert_eq!(fs::read_to_string(&second_dest).unwrap(), "2");
    }

    #[test]
    fn folder_is_moved_recursively() {
        let root = tempfile::tempdir().expect("temp dir");
        let archive_dir = root.path().join("archive");
        let cfg = config(archive_dir.clone(), SortMode::ByCategory);

        let folder = root.path().join("Project");
        write(&folder.join("notes.txt"), "notes");
        write(&folder.join("assets").join("logo.png"), "png");

        let destination = move_to_archive(&folder, &cfg).expect("move folder");

        assert!(!folder.exists(), "исходная папка должна исчезнуть");
        assert_eq!(destination, archive_dir.join("Other").join("Project"));
        assert_eq!(
            fs::read_to_string(destination.join("notes.txt")).unwrap(),
            "notes"
        );
        assert_eq!(
            fs::read_to_string(destination.join("assets").join("logo.png")).unwrap(),
            "png"
        );
    }

    #[test]
    fn cross_device_file_copy_is_verified_before_removing_source() {
        let root = tempfile::tempdir().expect("temp dir");
        let src = root.path().join("source.bin");
        let dst = root.path().join("nested").join("dest.bin");
        let contents = vec![42u8; 1024 * 16];
        fs::create_dir_all(dst.parent().unwrap()).unwrap();
        fs::write(&src, &contents).expect("write src");

        copy_file_verified(&src, &dst).expect("copy should succeed");

        let src_meta = fs::metadata(&src).expect("src metadata");
        let dst_meta = fs::metadata(&dst).expect("dst metadata");
        assert_eq!(src_meta.len(), dst_meta.len());
        assert_eq!(fs::read(&dst).unwrap(), contents);
        assert!(
            src.exists(),
            "источник должен оставаться на месте до явного удаления после успешной проверки"
        );

        // Имитируем ветку EXDEV: удаляем исходник только после успешной проверки копии.
        fs::remove_file(&src).expect("remove src");
        assert!(!src.exists());
        assert!(dst.exists());
    }

    #[test]
    fn cross_device_copy_failure_keeps_source_intact() {
        let root = tempfile::tempdir().expect("temp dir");
        let src = root.path().join("source.txt");
        write(&src, "important");
        let dst_dir = root.path().join("dst_as_dir");
        fs::create_dir_all(&dst_dir).unwrap();

        let err = copy_file_verified(&src, &dst_dir).expect_err("copy into directory must fail");

        assert!(src.exists(), "исходник нельзя удалять при неуспешной копии");
        assert_eq!(fs::read_to_string(&src).unwrap(), "important");
        assert!(!err.to_string().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_archive_survives_broken_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().expect("temp dir");
        let archive = root.path().join("archive");
        let images = archive.join("Images");
        fs::create_dir_all(&images).unwrap();
        symlink(archive.join("missing-target"), images.join("broken.png")).unwrap();

        let removed = cleanup_expired_archive_with(&archive, 7, |path| {
            fs::remove_file(path)
        })
        .expect("битая ссылка не должна ломать очистку");

        assert_eq!(removed, 0);
        assert!(images.join("broken.png").symlink_metadata().is_ok());
    }

    #[test]
    fn cleanup_expired_archive_trashes_old_items_and_removes_empty_date_dirs() {
        let root = tempfile::tempdir().expect("temp dir");
        let archive = root.path().join("archive");

        let old_dated = archive.join("2026-01-01").join("old.txt");
        let fresh_dated = archive.join("2026-01-01").join("fresh.txt");
        let old_only_day = archive.join("2026-01-02").join("old-only.txt");
        let old_image = archive.join("Images").join("old.png");
        let fresh_image = archive.join("Images").join("fresh.png");
        let old_root = archive.join("root-old.pdf");

        write(&old_dated, "old");
        write(&fresh_dated, "fresh");
        write(&old_only_day, "old");
        write(&old_image, "old");
        write(&fresh_image, "fresh");
        write(&old_root, "old");

        let old_time = SystemTime::now() - Duration::from_secs(10 * 86_400);
        for path in [&old_dated, &old_only_day, &old_image, &old_root] {
            fs::File::open(path)
                .unwrap()
                .set_modified(old_time)
                .expect("set mtime");
        }

        fn fake_delete(path: &Path) -> io::Result<()> {
            if path.is_dir() {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            }
        }

        let removed =
            cleanup_expired_archive_with(&archive, 7, fake_delete).expect("cleanup archive");

        assert_eq!(removed, 4);
        assert!(!old_dated.exists());
        assert!(!old_only_day.exists());
        assert!(!old_image.exists());
        assert!(!old_root.exists());
        assert!(fresh_dated.exists(), "свежие файлы должны остаться");
        assert!(fresh_image.exists(), "свежие файлы должны остаться");
        assert!(
            !archive.join("2026-01-02").exists(),
            "опустевшая папка даты должна быть удалена"
        );
        assert!(archive.join("2026-01-01").exists());
    }
}
