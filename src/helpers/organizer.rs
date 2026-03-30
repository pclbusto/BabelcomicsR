use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use sqlx::{Row, SqlitePool};

use crate::models::{ComicbookInfo, Publisher, Volume};
use crate::repositories::{
    ComicbookInfoRepository, ComicbookRepository, PublisherRepository, VolumeRepository,
};

// ── Tipos públicos ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrganizationStatus {
    Ok,
    Duplicate,
    AlreadyInPlace,
    NoInfo,
    Error,
}

impl OrganizationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "OK",
            Self::Duplicate => "DUPLICATE",
            Self::AlreadyInPlace => "ALREADY_IN_PLACE",
            Self::NoInfo => "NO_INFO",
            Self::Error => "ERROR",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ComicOrganizationPlan {
    pub comicbook_id: i64,
    pub current_path: String,
    pub new_path_relative: String,
    pub new_path_absolute: String,
    pub filename_normalized: String,
    pub status: OrganizationStatus,
    pub message: String,
    /// 0 = original, 1+ = versión duplicada
    pub version: u32,
    pub volume_id: i64,
    pub volume_name: String,
}

#[derive(Debug, Default)]
pub struct ExecuteStats {
    pub success: usize,
    pub failed: usize,
    pub skipped: usize,
}

// ── Struct interno de metadata ─────────────────────────────────────────────────

struct ComicMetadata {
    info: ComicbookInfo,
    volume: Volume,
    publisher: Option<Publisher>,
}

// ── ComicOrganizer ─────────────────────────────────────────────────────────────

pub struct ComicOrganizer {
    base_folder: PathBuf,
    pool: SqlitePool,
}

impl ComicOrganizer {
    pub fn new(base_folder: impl Into<PathBuf>, pool: SqlitePool) -> Self {
        Self {
            base_folder: base_folder.into(),
            pool,
        }
    }

    // ── Utilidades de nombre ───────────────────────────────────────────────────

    /// Limpia caracteres inválidos en nombres de archivo/carpeta.
    pub fn sanitize_filename(name: &str) -> String {
        let mut result = String::with_capacity(name.len());
        for c in name.chars() {
            match c {
                '/' | '\\' => result.push('-'),
                ':' => result.push('-'),
                '*' | '?' | '"' | '<' | '>' | '|' => {} // eliminar
                '\n' | '\r' | '\t' => result.push(' '),
                other => result.push(other),
            }
        }
        // Colapsar espacios múltiples
        let mut out = String::with_capacity(result.len());
        let mut prev_space = false;
        for c in result.chars() {
            if c == ' ' {
                if !prev_space {
                    out.push(' ');
                }
                prev_space = true;
            } else {
                out.push(c);
                prev_space = false;
            }
        }
        out.trim().to_string()
    }

    /// Genera nombre normalizado: `{Volume} {NNN}[_verXX].ext`
    pub fn generate_normalized_filename(
        volume_name: &str,
        issue_number: &str,
        extension: &str,
        version: u32,
    ) -> String {
        let clean_volume = Self::sanitize_filename(volume_name);
        let formatted_number = first_number_in(issue_number)
            .map(|n| format!("{:03}", n))
            .unwrap_or_else(|| Self::sanitize_filename(issue_number));

        let mut base = format!("{} {}", clean_volume, formatted_number);
        if version > 0 {
            base.push_str(&format!("_ver{:02}", version));
        }
        format!("{}{}", base, extension)
    }

    /// Detecta el tipo real del archivo con el comando `file` y corrige la extensión si hace falta.
    fn detect_and_fix_extension(file_path: &Path, original_ext: &str) -> String {
        let Ok(output) = std::process::Command::new("file")
            .args(["-b", "--mime-type", &file_path.to_string_lossy()])
            .output()
        else {
            return original_ext.to_string();
        };

        let mime = String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_lowercase();

        let correct_ext = if mime.contains("zip") {
            ".cbz"
        } else if mime.contains("rar") {
            ".cbr"
        } else if mime.contains("7z") {
            ".cb7"
        } else if mime.contains("pdf") {
            ".pdf"
        } else {
            return original_ext.to_string();
        };

        if correct_ext != original_ext.to_lowercase().as_str() {
            tracing::warn!(
                "Extensión corregida: {} → {} (tipo: {}) — {}",
                original_ext,
                correct_ext,
                mime,
                file_path.display()
            );
            correct_ext.to_string()
        } else {
            original_ext.to_string()
        }
    }

    // ── Metadata desde BD ─────────────────────────────────────────────────────

    async fn get_comic_metadata(&self, comicbook_id: i64) -> Result<Option<ComicMetadata>> {
        let cb = match ComicbookRepository::new(&self.pool)
            .get_by_id(comicbook_id)
            .await?
        {
            Some(c) => c,
            None => return Ok(None),
        };

        let info_id = match cb.id_comicbook_info {
            Some(id) => id,
            None => return Ok(None),
        };

        let info = match ComicbookInfoRepository::new(&self.pool)
            .get_by_id(info_id)
            .await?
        {
            Some(i) => i,
            None => return Ok(None),
        };

        let volume_id = match info.id_volume {
            Some(id) => id,
            None => return Ok(None),
        };

        let volume = match VolumeRepository::new(&self.pool)
            .get_by_id(volume_id)
            .await?
        {
            Some(v) => v,
            None => return Ok(None),
        };

        let publisher = if volume.id_publisher > 0 {
            PublisherRepository::new(&self.pool).get_by_id(volume.id_publisher).await?
        } else {
            None
        };

        Ok(Some(ComicMetadata {
            info,
            volume,
            publisher,
        }))
    }

    // ── Paths ─────────────────────────────────────────────────────────────────

    /// Genera el path relativo y absoluto de destino.
    /// Estructura: `{Publisher}/{Volume[-Year]}/{filename}`
    pub fn generate_target_path(
        &self,
        publisher_name: Option<&str>,
        volume_name: &str,
        volume_year: Option<i64>,
        filename: &str,
    ) -> (String, String) {
        let publisher_folder = publisher_name
            .map(|n| Self::sanitize_filename(n))
            .unwrap_or_else(|| "Sin Editorial".to_string());

        let volume_folder = match volume_year.filter(|&y| y > 0) {
            Some(year) => Self::sanitize_filename(&format!("{}-{}", volume_name, year)),
            None => Self::sanitize_filename(volume_name),
        };

        let relative = format!("{}/{}/{}", publisher_folder, volume_folder, filename);
        let absolute = self.base_folder.join(&relative).to_string_lossy().to_string();
        (relative, absolute)
    }

    /// Encuentra versiones existentes de un archivo base (0 = original, 1+ = `_verXX`).
    pub fn find_existing_versions(target_dir: &Path, base_filename: &str) -> Vec<u32> {
        if !target_dir.exists() {
            return vec![];
        }

        let dot = base_filename.rfind('.').unwrap_or(base_filename.len());
        let name = &base_filename[..dot];
        let ext = &base_filename[dot..];
        let ver_prefix = format!("{}_ver", name);

        let mut versions = Vec::new();
        if target_dir.join(base_filename).exists() {
            versions.push(0);
        }

        if let Ok(entries) = std::fs::read_dir(target_dir) {
            for entry in entries.flatten() {
                let fname = entry.file_name().to_string_lossy().to_string();
                if fname.starts_with(&ver_prefix) && fname.ends_with(ext) {
                    let mid = &fname[ver_prefix.len()..fname.len() - ext.len()];
                    if let Ok(n) = mid.parse::<u32>() {
                        versions.push(n);
                    }
                }
            }
        }

        versions.sort_unstable();
        versions
    }

    // ── Creación de planes ────────────────────────────────────────────────────

    /// Crea un plan de reorganización para los comics indicados.
    /// - `volume_ids`: filtra por volumen(es); si es `None`, procesa todos.
    /// - `comicbook_ids`: filtra por IDs específicos de comic.
    pub async fn create_organization_plan(
        &self,
        volume_ids: Option<&[i64]>,
        comicbook_ids: Option<&[i64]>,
    ) -> Result<Vec<ComicOrganizationPlan>> {
        let rows = self
            .fetch_comicbooks_for_plan(volume_ids, comicbook_ids)
            .await?;

        let mut plans = Vec::with_capacity(rows.len());

        for (id, path) in rows {
            match self.create_plan_for_comic(id, &path).await {
                Ok(plan) => plans.push(plan),
                Err(e) => {
                    tracing::error!("Error creando plan para comic {}: {}", id, e);
                    plans.push(ComicOrganizationPlan {
                        comicbook_id: id,
                        current_path: path,
                        new_path_relative: String::new(),
                        new_path_absolute: String::new(),
                        filename_normalized: String::new(),
                        status: OrganizationStatus::Error,
                        message: format!("Error al generar plan: {}", e),
                        version: 0,
                        volume_id: 0,
                        volume_name: String::new(),
                    });
                }
            }
        }

        self.resolve_duplicate_plans(&mut plans).await?;
        Ok(plans)
    }

    async fn fetch_comicbooks_for_plan(
        &self,
        volume_ids: Option<&[i64]>,
        comicbook_ids: Option<&[i64]>,
    ) -> Result<Vec<(i64, String)>> {
        let rows = if let Some(vids) = volume_ids {
            if vids.is_empty() {
                return Ok(vec![]);
            }
            let placeholders = vids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                r#"SELECT cb.id_comicbook, cb.path
                   FROM comicbooks cb
                   JOIN comicbooks_info ci ON cb.id_comicbook_info = ci.id_comicbook_info
                   WHERE cb.en_papelera = 0
                     AND ci.id_volume IN ({})
                   ORDER BY cb.path"#,
                placeholders
            );
            let mut q = sqlx::query(&sql);
            for id in vids {
                q = q.bind(id);
            }
            q.fetch_all(&self.pool).await?
        } else if let Some(cbids) = comicbook_ids {
            if cbids.is_empty() {
                return Ok(vec![]);
            }
            let placeholders = cbids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!(
                r#"SELECT id_comicbook, path FROM comicbooks
                   WHERE en_papelera = 0 AND id_comicbook IN ({})
                   ORDER BY path"#,
                placeholders
            );
            let mut q = sqlx::query(&sql);
            for id in cbids {
                q = q.bind(id);
            }
            q.fetch_all(&self.pool).await?
        } else {
            sqlx::query(
                "SELECT id_comicbook, path FROM comicbooks WHERE en_papelera = 0 ORDER BY path",
            )
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows
            .into_iter()
            .map(|r| (r.get::<i64, _>(0), r.get::<String, _>(1)))
            .collect())
    }

    async fn create_plan_for_comic(
        &self,
        comicbook_id: i64,
        path: &str,
    ) -> Result<ComicOrganizationPlan> {
        let current_path = make_absolute(Path::new(path), &self.base_folder);
        let current_path_str = current_path.to_string_lossy().to_string();

        let meta = match self.get_comic_metadata(comicbook_id).await? {
            None => {
                return Ok(ComicOrganizationPlan {
                    comicbook_id,
                    current_path: current_path_str,
                    new_path_relative: String::new(),
                    new_path_absolute: String::new(),
                    filename_normalized: String::new(),
                    status: OrganizationStatus::NoInfo,
                    message: "Cómic sin catalogar (no tiene ComicbookInfo asociado)".to_string(),
                    version: 0,
                    volume_id: 0,
                    volume_name: String::new(),
                });
            }
            Some(m) => m,
        };

        let publisher_name = meta.publisher.as_ref().map(|p| p.nombre.as_str());
        let volume_name = &meta.volume.nombre;
        let issue_number = meta.info.numero.as_deref().unwrap_or("0");

        let orig_ext = current_path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
            .unwrap_or_default();
        let extension = Self::detect_and_fix_extension(&current_path, &orig_ext);

        let base_filename =
            Self::generate_normalized_filename(volume_name, issue_number, &extension, 0);
        let (relative, absolute) = self.generate_target_path(
            publisher_name,
            volume_name,
            Some(meta.volume.anio_inicio),
            &base_filename,
        );

        // ¿Ya está en el lugar correcto?
        if make_absolute(Path::new(&current_path_str), &self.base_folder)
            == make_absolute(Path::new(&absolute), &self.base_folder)
        {
            return Ok(ComicOrganizationPlan {
                comicbook_id,
                current_path: current_path_str,
                new_path_relative: relative,
                new_path_absolute: absolute,
                filename_normalized: base_filename,
                status: OrganizationStatus::AlreadyInPlace,
                message: "Ya está en la ubicación correcta".to_string(),
                version: 0,
                volume_id: meta.volume.id_volume,
                volume_name: volume_name.clone(),
            });
        }

        // ¿Hay duplicados en destino?
        let target_dir = PathBuf::from(&absolute);
        let target_dir = target_dir
            .parent()
            .unwrap_or(Path::new(""))
            .to_path_buf();
        let existing = Self::find_existing_versions(&target_dir, &base_filename);

        if existing.is_empty() {
            Ok(ComicOrganizationPlan {
                comicbook_id,
                current_path: current_path_str,
                new_path_relative: relative,
                new_path_absolute: absolute,
                filename_normalized: base_filename,
                status: OrganizationStatus::Ok,
                message: "Listo para mover".to_string(),
                version: 0,
                volume_id: meta.volume.id_volume,
                volume_name: volume_name.clone(),
            })
        } else {
            let next = existing.iter().max().copied().unwrap_or(0) + 1;
            let versioned =
                Self::generate_normalized_filename(volume_name, issue_number, &extension, next);
            let (rel_v, abs_v) = self.generate_target_path(
                publisher_name,
                volume_name,
                Some(meta.volume.anio_inicio),
                &versioned,
            );
            Ok(ComicOrganizationPlan {
                comicbook_id,
                current_path: current_path_str,
                new_path_relative: rel_v,
                new_path_absolute: abs_v,
                filename_normalized: versioned,
                status: OrganizationStatus::Duplicate,
                message: format!("Duplicado detectado, se creará versión {:02}", next),
                version: next,
                volume_id: meta.volume.id_volume,
                volume_name: volume_name.clone(),
            })
        }
    }

    /// Segundo pase: detecta planes que apuntan al mismo destino y les asigna versiones.
    async fn resolve_duplicate_plans(
        &self,
        plans: &mut Vec<ComicOrganizationPlan>,
    ) -> Result<()> {
        // Agrupar índices por path de destino
        let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, plan) in plans.iter().enumerate() {
            if matches!(
                plan.status,
                OrganizationStatus::Ok | OrganizationStatus::Duplicate
            ) {
                groups
                    .entry(plan.new_path_absolute.clone())
                    .or_default()
                    .push(i);
            }
        }

        for (target_path, indices) in groups {
            if indices.len() <= 1 {
                continue;
            }

            let target = PathBuf::from(&target_path);
            let target_dir = target.parent().unwrap_or(Path::new("")).to_path_buf();
            let base_filename = target
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            let existing = Self::find_existing_versions(&target_dir, &base_filename);
            let (start_version, skip_first) = if existing.is_empty() {
                (1u32, true)
            } else {
                (existing.iter().max().copied().unwrap_or(0) + 1, false)
            };

            for (batch_idx, &plan_idx) in indices.iter().enumerate() {
                if batch_idx == 0 && skip_first {
                    continue;
                }
                let version = if skip_first {
                    start_version + batch_idx as u32 - 1
                } else {
                    start_version + batch_idx as u32
                };

                let comicbook_id = plans[plan_idx].comicbook_id;
                let current_path = plans[plan_idx].current_path.clone();

                let meta = match self.get_comic_metadata(comicbook_id).await? {
                    Some(m) => m,
                    None => continue,
                };

                let ext = Path::new(&current_path)
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
                    .unwrap_or_default();

                let versioned = Self::generate_normalized_filename(
                    &meta.volume.nombre,
                    meta.info.numero.as_deref().unwrap_or("0"),
                    &ext,
                    version,
                );
                let (rel, abs) = self.generate_target_path(
                    meta.publisher.as_ref().map(|p| p.nombre.as_str()),
                    &meta.volume.nombre,
                    Some(meta.volume.anio_inicio),
                    &versioned,
                );

                plans[plan_idx].new_path_relative = rel;
                plans[plan_idx].new_path_absolute = abs;
                plans[plan_idx].filename_normalized = versioned;
                plans[plan_idx].status = OrganizationStatus::Duplicate;
                plans[plan_idx].message =
                    format!("Duplicado en el lote, se creará versión {:02}", version);
                plans[plan_idx].version = version;
            }
        }

        Ok(())
    }

    // ── Ejecución ─────────────────────────────────────────────────────────────

    /// Ejecuta un plan individual. Devuelve `Ok(mensaje)` o `Err(mensaje)`.
    pub async fn execute_plan(
        &self,
        plan: &ComicOrganizationPlan,
        dry_run: bool,
    ) -> Result<String, String> {
        if matches!(
            plan.status,
            OrganizationStatus::NoInfo
                | OrganizationStatus::AlreadyInPlace
                | OrganizationStatus::Error
        ) {
            return Err(format!("No se puede ejecutar: {}", plan.message));
        }

        if !Path::new(&plan.current_path).exists() {
            return Err(format!(
                "Archivo origen no existe: {}",
                plan.current_path
            ));
        }

        if dry_run {
            return Ok(format!(
                "[DRY RUN] Movería: {} → {}",
                plan.current_path, plan.new_path_absolute
            ));
        }

        // Crear directorio de destino
        let target_dir = Path::new(&plan.new_path_absolute)
            .parent()
            .unwrap_or(Path::new(""));
        std::fs::create_dir_all(target_dir)
            .map_err(|e| format!("Error creando directorio destino: {}", e))?;

        // Mover archivo (con fallback copy+delete para cross-device)
        move_file(
            Path::new(&plan.current_path),
            Path::new(&plan.new_path_absolute),
        )
        .map_err(|e| format!("Error moviendo archivo: {}", e))?;

        // Actualizar path en BD
        sqlx::query("UPDATE comicbooks SET path = ? WHERE id_comicbook = ?")
            .bind(&plan.new_path_absolute)
            .bind(plan.comicbook_id)
            .execute(&self.pool)
            .await
            .map_err(|e| {
                tracing::error!(
                    "Archivo movido pero error actualizando BD para comic {}: {}",
                    plan.comicbook_id,
                    e
                );
                format!("Archivo movido pero error en BD: {}", e)
            })?;

        Ok(format!("Movido: {}", plan.filename_normalized))
    }

    /// Ejecuta una lista de planes, llamando `on_progress` tras cada uno.
    pub async fn execute_plans<F>(
        &self,
        plans: &[ComicOrganizationPlan],
        dry_run: bool,
        mut on_progress: F,
    ) -> ExecuteStats
    where
        F: FnMut(usize, usize, &ComicOrganizationPlan, bool, &str),
    {
        let mut stats = ExecuteStats::default();
        let total = plans.len();

        for (i, plan) in plans.iter().enumerate() {
            if matches!(
                plan.status,
                OrganizationStatus::NoInfo
                    | OrganizationStatus::AlreadyInPlace
                    | OrganizationStatus::Error
            ) {
                stats.skipped += 1;
                on_progress(i, total, plan, false, &plan.message);
                continue;
            }

            match self.execute_plan(plan, dry_run).await {
                Ok(msg) => {
                    stats.success += 1;
                    on_progress(i, total, plan, true, &msg);
                }
                Err(msg) => {
                    stats.failed += 1;
                    on_progress(i, total, plan, false, &msg);
                }
            }
        }

        stats
    }
}

// ── Helpers privados ───────────────────────────────────────────────────────────

/// Primer número encontrado en la cadena (equiv. a `re.search(r'(\d+)', s)`).
fn first_number_in(s: &str) -> Option<u32> {
    let mut start: Option<usize> = None;
    for (i, c) in s.char_indices() {
        if c.is_ascii_digit() {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(si) = start {
            return s[si..i].parse().ok();
        }
    }
    start.and_then(|si| s[si..].parse().ok())
}

/// Convierte un path relativo en absoluto usando `fallback_base`.
/// No resuelve symlinks (equiv. a Python's `Path.absolute()`).
fn make_absolute(path: &Path, fallback_base: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        fallback_base.join(path)
    }
}

/// Mueve un archivo manejando el caso cross-device (copy + delete).
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Err(e) = std::fs::rename(from, to) {
        if e.kind() == std::io::ErrorKind::CrossesDevices {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)?;
        } else {
            return Err(e);
        }
    }
    Ok(())
}
