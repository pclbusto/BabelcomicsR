use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ProgressSource {
    pub path: String,
    pub size_bytes: u64,
    pub modified_unix_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ReadingProgress {
    pub schema_version: u32,
    pub comicbook_id: i64,
    pub source: ProgressSource,
    pub total_pages: usize,
    pub last_page_index: Option<usize>,
    pub visited_page_indexes: Vec<usize>,
    pub completed_at: Option<String>,
    pub updated_at: String,
}

impl Default for ReadingProgress {
    fn default() -> Self {
        Self {
            schema_version: 1,
            comicbook_id: 0,
            source: ProgressSource::default(),
            total_pages: 0,
            last_page_index: None,
            visited_page_indexes: Vec::new(),
            completed_at: None,
            updated_at: String::new(),
        }
    }
}

#[derive(Debug)]
pub struct ProgressTracker {
    path: PathBuf,
    comic_path: PathBuf,
    document: ReadingProgress,
}

impl ProgressTracker {
    pub fn open(path: PathBuf, comic_path: PathBuf, total_pages: usize) -> Result<Self, String> {
        let mut document = if path.exists() {
            let contents = fs::read_to_string(&path)
                .map_err(|error| format!("No se pudo leer {}: {error}", path.display()))?;
            serde_json::from_str(&contents)
                .map_err(|error| format!("El progreso {} no es válido: {error}", path.display()))?
        } else {
            ReadingProgress::default()
        };

        document.schema_version = 1;
        document.total_pages = total_pages;
        document
            .visited_page_indexes
            .retain(|index| *index < total_pages);
        document.visited_page_indexes.sort_unstable();
        document.visited_page_indexes.dedup();
        document.last_page_index = document
            .last_page_index
            .filter(|index| *index < total_pages);
        if document.visited_page_indexes.len() < total_pages {
            document.completed_at = None;
        }

        let mut tracker = Self {
            path,
            comic_path,
            document,
        };
        tracker.refresh_source();
        Ok(tracker)
    }

    pub fn resume_page_index(&self) -> usize {
        self.document
            .last_page_index
            .unwrap_or(0)
            .min(self.document.total_pages.saturating_sub(1))
    }

    pub fn visit(&mut self, page_index: usize) -> Result<(), String> {
        if page_index >= self.document.total_pages {
            return Ok(());
        }

        self.document.last_page_index = Some(page_index);
        let mut visited = self
            .document
            .visited_page_indexes
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();
        visited.insert(page_index);
        self.document.visited_page_indexes = visited.into_iter().collect();
        let updated_at = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .map_err(|error| format!("No se pudo generar la fecha del progreso: {error}"))?;
        self.document.updated_at.clone_from(&updated_at);
        if self.document.total_pages > 0
            && self.document.visited_page_indexes.len() == self.document.total_pages
            && self.document.completed_at.is_none()
        {
            self.document.completed_at = Some(updated_at);
        }
        self.refresh_source();
        self.save_atomically()
    }

    /// Actualiza tamaño y fecha del cómic tras modificarlo (por ejemplo al
    /// guardar la caché de miniaturas), para que Babelcomics no lo considere
    /// otro archivo y conserve el progreso.
    pub fn refresh_after_archive_change(&mut self) -> Result<(), String> {
        self.refresh_source();
        self.save_atomically()
    }

    fn refresh_source(&mut self) {
        let canonical_path = self
            .comic_path
            .canonicalize()
            .unwrap_or_else(|_| self.comic_path.clone());
        self.document.source.path = canonical_path.to_string_lossy().into_owned();

        if let Ok(metadata) = fs::metadata(&self.comic_path) {
            self.document.source.size_bytes = metadata.len();
            self.document.source.modified_unix_ms = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
                .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
                .unwrap_or(0);
        }
    }

    fn save_atomically(&self) -> Result<(), String> {
        if let Some(parent) = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "No se pudo crear la carpeta de progreso {}: {error}",
                    parent.display()
                )
            })?;
        }

        let json = serde_json::to_string_pretty(&self.document)
            .map_err(|error| format!("No se pudo serializar el progreso: {error}"))?;
        let temporary_path = temporary_path_for(&self.path);

        let write_result = (|| {
            let mut file = File::create(&temporary_path).map_err(|error| {
                format!("No se pudo crear {}: {error}", temporary_path.display())
            })?;
            file.write_all(json.as_bytes())
                .and_then(|_| file.write_all(b"\n"))
                .and_then(|_| file.sync_all())
                .map_err(|error| {
                    format!("No se pudo escribir {}: {error}", temporary_path.display())
                })?;
            fs::rename(&temporary_path, &self.path)
                .map_err(|error| format!("No se pudo actualizar {}: {error}", self.path.display()))
        })();

        if write_result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        write_result
    }
}

fn temporary_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_else(|| "reading-progress.json".into());
    name.push(format!(".{}.tmp", std::process::id()));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn visit_keeps_pages_sorted_and_unique() {
        let unique = format!(
            "cosmic-reader-progress-{}-{}.json",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let progress_path = std::env::temp_dir().join(unique);
        let comic_path = std::env::current_exe().unwrap();
        let mut tracker = ProgressTracker::open(progress_path.clone(), comic_path, 12).unwrap();

        tracker.visit(9).unwrap();
        tracker.visit(1).unwrap();
        tracker.visit(9).unwrap();

        let saved: ReadingProgress =
            serde_json::from_str(&fs::read_to_string(&progress_path).unwrap()).unwrap();
        assert_eq!(saved.last_page_index, Some(9));
        assert_eq!(saved.visited_page_indexes, vec![1, 9]);
        assert_eq!(saved.total_pages, 12);
        assert_eq!(saved.schema_version, 1);
        assert!(!saved.updated_at.is_empty());
        assert!(saved.completed_at.is_none());

        for page_index in 0..12 {
            tracker.visit(page_index).unwrap();
        }
        let completed: ReadingProgress =
            serde_json::from_str(&fs::read_to_string(&progress_path).unwrap()).unwrap();
        assert!(completed.completed_at.is_some());

        fs::remove_file(progress_path).unwrap();
    }
}
