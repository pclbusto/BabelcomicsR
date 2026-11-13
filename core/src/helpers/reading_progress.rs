use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};

use crate::models::{ReadingProgress, ReadingProgressSource};

pub fn spool_dir() -> PathBuf {
    if let Some(state_home) = std::env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(state_home)
            .join("babelcomics")
            .join("reading-progress")
            .join("pending");
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".local/state/babelcomics/reading-progress/pending")
}

pub fn spool_path(comicbook_id: i64) -> PathBuf {
    spool_dir().join(format!("{comicbook_id}.json"))
}

pub fn source_for(comic_path: &Path) -> ReadingProgressSource {
    let canonical = comic_path
        .canonicalize()
        .unwrap_or_else(|_| comic_path.to_path_buf());
    let mut source = ReadingProgressSource {
        path: canonical.to_string_lossy().into_owned(),
        ..ReadingProgressSource::default()
    };
    if let Ok(metadata) = fs::metadata(comic_path) {
        source.size_bytes = metadata.len();
        source.modified_unix_ms = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
            .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
            .unwrap_or(0);
    }
    source
}

pub fn load(path: &Path) -> Result<ReadingProgress> {
    let contents =
        fs::read_to_string(path).with_context(|| format!("No se pudo leer {}", path.display()))?;
    let mut progress: ReadingProgress = serde_json::from_str(&contents)
        .with_context(|| format!("El progreso {} no es válido", path.display()))?;
    progress.normalize();
    Ok(progress)
}

pub fn save_atomically(path: &Path, progress: &ReadingProgress) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("No se pudo crear {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(progress)?;
    let temporary = temporary_path_for(path);
    let result = (|| -> Result<()> {
        let mut file = File::create(&temporary)
            .with_context(|| format!("No se pudo crear {}", temporary.display()))?;
        file.write_all(json.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)
            .with_context(|| format!("No se pudo actualizar {}", path.display()))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

pub fn pending_files() -> Result<Vec<PathBuf>> {
    let directory = spool_dir();
    if !directory.exists() {
        return Ok(Vec::new());
    }
    let mut files = fs::read_dir(&directory)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}

fn temporary_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "reading-progress.json".into());
    name.push(format!(".{}.tmp", std::process::id()));
    path.with_file_name(name)
}
