use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};

use super::scanner::ComicFormat;

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp"];

// ---------------------------------------------------------------------------
// API lazy (usada por el lector)
// ---------------------------------------------------------------------------

/// Lista todas las páginas de un cómic sin extraer nada.
/// Devuelve los nombres ordenados naturalmente.
/// Para PDF devuelve strings de la forma "0001", "0002", etc.
pub fn list_pages(path: &str) -> Result<Vec<String>> {
    let p = Path::new(path);
    match ComicFormat::from_path(p) {
        ComicFormat::Cbz => list_pages_zip(p),
        ComicFormat::Cbr => list_pages_rar(p),
        ComicFormat::Cb7 => list_pages_7z(p),
        ComicFormat::Pdf => list_pages_pdf(p),
        ComicFormat::Unknown => bail!("Formato no soportado: {}", path),
    }
}

/// Extrae una sola página al `target_dir`.
/// Si el archivo ya existe en `target_dir` lo devuelve directamente.
/// Para CB7 la primera llamada extrae todo el archivo (limitación del formato);
/// las siguientes llamadas son instantáneas gracias al caché en disco.
pub fn extract_single_page(comic_path: &str, page_name: &str, target_dir: &Path) -> Result<PathBuf> {
    if !target_dir.exists() {
        std::fs::create_dir_all(target_dir)?;
    }

    let file_name = Path::new(page_name).file_name().unwrap_or_default();
    let dest = target_dir.join(file_name);
    if dest.exists() {
        return Ok(dest);
    }

    let p = Path::new(comic_path);
    match ComicFormat::from_path(p) {
        ComicFormat::Cbz => extract_page_zip(p, page_name, target_dir),
        ComicFormat::Cbr => extract_page_rar(p, page_name, target_dir),
        ComicFormat::Cb7 => extract_page_7z(p, page_name, target_dir),
        ComicFormat::Pdf => extract_page_pdf(p, page_name, target_dir),
        ComicFormat::Unknown => bail!("Formato no soportado: {}", comic_path),
    }
}

/// Extrae una sola página directamente a memoria.
pub fn extract_page_to_memory(comic_path: &str, page_name: &str) -> Result<Vec<u8>> {
    let p = Path::new(comic_path);
    match ComicFormat::from_path(p) {
        ComicFormat::Cbz => extract_page_to_memory_zip(p, page_name),
        ComicFormat::Cbr => extract_page_to_memory_rar(p, page_name),
        ComicFormat::Cb7 => extract_page_to_memory_7z(p, page_name),
        ComicFormat::Pdf => extract_page_to_memory_pdf(p, page_name),
        ComicFormat::Unknown => bail!("Formato no soportado: {}", comic_path),
    }
}

// --- list helpers ---

fn list_pages_zip(path: &Path) -> Result<Vec<String>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut names: Vec<String> = (0..archive.len())
        .filter_map(|i| {
            archive.by_index(i).ok().and_then(|f| {
                let name = f.name().to_string();
                if is_image_name(&name.to_lowercase()) { Some(name) } else { None }
            })
        })
        .collect();
    names.sort_by(|a, b| {
        let a_base = Path::new(a).file_name().unwrap_or_default().to_string_lossy().into_owned();
        let b_base = Path::new(b).file_name().unwrap_or_default().to_string_lossy().into_owned();
        natural_sort_compare(&a_base, &b_base)
    });
    Ok(names)
}

fn list_pages_rar(path: &Path) -> Result<Vec<String>> {
    let output = Command::new("unrar")
        .args(["lb", path.to_str().unwrap_or("")])
        .output()
        .context("No se pudo ejecutar 'unrar'")?;
    let listing = String::from_utf8_lossy(&output.stdout);
    let mut names: Vec<String> = listing
        .lines()
        .filter(|l| is_image_name(&l.to_lowercase()))
        .map(|l| l.to_string())
        .collect();
    names.sort_by(|a, b| {
        let a_base = Path::new(a).file_name().unwrap_or_default().to_string_lossy().into_owned();
        let b_base = Path::new(b).file_name().unwrap_or_default().to_string_lossy().into_owned();
        natural_sort_compare(&a_base, &b_base)
    });
    Ok(names)
}

fn list_pages_7z(path: &Path) -> Result<Vec<String>> {
    use sevenz_rust::Archive;
    use std::io::BufReader;

    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let archive = Archive::read(&mut reader, path.metadata()?.len(), &[])
        .with_context(|| format!("7Z inválido: {}", path.display()))?;

    let mut names: Vec<String> = archive
        .files
        .iter()
        .filter(|f| !f.is_directory() && is_image_name(&f.name().to_lowercase()))
        .map(|f| f.name().to_string())
        .collect();

    names.sort_by(|a, b| {
        let a_base = Path::new(a).file_name().unwrap_or_default().to_string_lossy().into_owned();
        let b_base = Path::new(b).file_name().unwrap_or_default().to_string_lossy().into_owned();
        natural_sort_compare(&a_base, &b_base)
    });
    Ok(names)
}

fn list_pages_pdf(path: &Path) -> Result<Vec<String>> {
    // Usa pdfinfo para obtener el número de páginas sin renderizar nada
    let output = Command::new("pdfinfo")
        .arg(path.to_str().unwrap_or(""))
        .output()
        .context("No se pudo ejecutar 'pdfinfo' (instala poppler-utils)")?;

    let text = String::from_utf8_lossy(&output.stdout);
    let count: usize = text
        .lines()
        .find(|l| l.starts_with("Pages:"))
        .and_then(|l| l.split_whitespace().last())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0);

    if count == 0 {
        bail!("No se pudo determinar el número de páginas del PDF: {}", path.display());
    }

    Ok((1..=count).map(|i| format!("{:04}", i)).collect())
}

// --- extract single memory helpers ---

fn extract_page_to_memory_zip(path: &Path, page_name: &str) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut entry = archive.by_name(page_name)?;
    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn extract_page_to_memory_rar(path: &Path, page_name: &str) -> Result<Vec<u8>> {
    let output = Command::new("unrar")
        .args(["p", "-inul", path.to_str().unwrap_or(""), page_name])
        .output()
        .context("No se pudo ejecutar 'unrar'")?;
    if !output.status.success() {
        bail!("unrar p falló extrayendo '{}' a memoria", page_name);
    }
    Ok(output.stdout)
}

fn extract_page_to_memory_7z(path: &Path, page_name: &str) -> Result<Vec<u8>> {
    // 7z no soporta bien extracción a stdout de un solo archivo con la lib actual
    // Reutilizamos la lógica de cover que usa un temporal pequeño y luego lo borra,
    // o bien extraemos todo si es CB7 (por eficiencia de acceso posterior).
    let tmp_dir = tempfile_dir()?;
    let extracted = extract_page_7z(path, page_name, Path::new(&tmp_dir))?;
    let bytes = std::fs::read(&extracted)?;
    let _ = std::fs::remove_file(&extracted);
    Ok(bytes)
}

fn extract_page_to_memory_pdf(path: &Path, page_name: &str) -> Result<Vec<u8>> {
    let page_num: u32 = page_name.parse().context("Número de página PDF inválido")?;
    let output = Command::new("pdftoppm")
        .args([
            "-f", &page_num.to_string(),
            "-l", &page_num.to_string(),
            "-jpeg",
            "-singlefile",
            path.to_str().unwrap_or(""),
        ])
        .output()
        .context("No se pudo ejecutar 'pdftoppm'")?;
    
    if !output.status.success() {
        bail!("pdftoppm falló para página {}", page_num);
    }
    Ok(output.stdout)
}

// --- extract single helpers ---

fn extract_page_zip(comic_path: &Path, page_name: &str, target_dir: &Path) -> Result<PathBuf> {
    let file = std::fs::File::open(comic_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    let mut entry = archive
        .by_name(page_name)
        .with_context(|| format!("No se encontró '{}' en el ZIP", page_name))?;
    let file_name = Path::new(page_name).file_name().unwrap_or_default();
    let outpath = target_dir.join(file_name);
    let mut outfile = std::fs::File::create(&outpath)?;
    std::io::copy(&mut entry, &mut outfile)?;
    Ok(outpath)
}

fn extract_page_rar(comic_path: &Path, page_name: &str, target_dir: &Path) -> Result<PathBuf> {
    let status = Command::new("unrar")
        .args([
            "e", "-y", "-o+",
            comic_path.to_str().unwrap_or(""),
            page_name,
            target_dir.to_str().unwrap_or(""),
        ])
        .status()?;
    if !status.success() {
        bail!("unrar falló extrayendo '{}'", page_name);
    }
    let file_name = Path::new(page_name).file_name().unwrap_or_default();
    Ok(target_dir.join(file_name))
}

/// Para CB7 no hay acceso aleatorio eficiente: la primera llamada extrae todo
/// el archivo. Las siguientes son instantáneas porque el archivo ya existe.
fn extract_page_7z(comic_path: &Path, _page_name: &str, target_dir: &Path) -> Result<PathBuf> {
    // Extrae todo el archivo de una vez; el caché en disco evita re-extracciones.
    sevenz_rust::decompress_file(comic_path, target_dir)
        .map_err(|e| anyhow::anyhow!("Error descomprimiendo 7Z: {}", e))?;

    let file_name = Path::new(_page_name).file_name().unwrap_or_default();
    let dest = target_dir.join(file_name);
    if dest.exists() {
        return Ok(dest);
    }
    // El archivo puede estar en un subdirectorio dentro del 7Z
    find_file_in_dir(target_dir.to_str().unwrap_or(""), &file_name.to_string_lossy())
}

fn extract_page_pdf(comic_path: &Path, page_name: &str, target_dir: &Path) -> Result<PathBuf> {
    // page_name es "0001", "0002", etc. (índice base-1)
    let page_num: u32 = page_name.parse().context("Número de página PDF inválido")?;
    let output_prefix = target_dir.join(format!("page-{:04}", page_num));
    let status = Command::new("pdftoppm")
        .args([
            "-f", &page_num.to_string(),
            "-l", &page_num.to_string(),
            "-jpeg",
            "-singlefile",
            comic_path.to_str().unwrap_or(""),
            output_prefix.to_str().unwrap_or(""),
        ])
        .status()
        .context("No se pudo ejecutar 'pdftoppm'")?;
    if !status.success() {
        bail!("pdftoppm falló extrayendo página {} de {}", page_num, comic_path.display());
    }
    let dest = target_dir.join(format!("page-{:04}.jpg", page_num));
    Ok(dest)
}

// ---------------------------------------------------------------------------
// API de extracción total (usada por el escáner de biblioteca)
// ---------------------------------------------------------------------------

/// Extrae todas las páginas de un cómic a un directorio de destino.
/// Devuelve una lista de rutas de archivos de imagen ordenadas naturalmente.
pub fn extract_all_pages(path: &str, target_dir: &Path) -> Result<Vec<PathBuf>> {
    let p = Path::new(path);
    if !target_dir.exists() {
        std::fs::create_dir_all(target_dir)?;
    }

    match ComicFormat::from_path(p) {
        ComicFormat::Cbz => extract_all_zip(p, target_dir),
        ComicFormat::Cbr => extract_all_rar(p, target_dir),
        ComicFormat::Cb7 => extract_all_7z(p, target_dir),
        ComicFormat::Pdf => extract_all_pdf(p, target_dir),
        ComicFormat::Unknown => bail!("Formato no soportado: {}", path),
    }
}

fn extract_all_zip(path: &Path, target_dir: &Path) -> Result<Vec<PathBuf>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file)?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_lowercase();
        if is_image_name(&name) {
            let outpath = target_dir.join(file.mangled_name());
            if let Some(p) = outpath.parent() {
                if !p.exists() { std::fs::create_dir_all(p)?; }
            }
            let mut outfile = std::fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
    }

    collect_and_sort_images(target_dir)
}

fn extract_all_rar(path: &Path, target_dir: &Path) -> Result<Vec<PathBuf>> {
    let status = Command::new("unrar")
        .args(["e", "-y", "-o+", path.to_str().unwrap_or(""), target_dir.to_str().unwrap_or("")])
        .status()?;

    if !status.success() {
        bail!("unrar falló extrayendo {}", path.display());
    }

    collect_and_sort_images(target_dir)
}

fn extract_all_7z(path: &Path, target_dir: &Path) -> Result<Vec<PathBuf>> {
    sevenz_rust::decompress_file(path, target_dir)
        .map_err(|e| anyhow::anyhow!("Error descomprimiendo 7Z: {}", e))?;

    collect_and_sort_images(target_dir)
}

fn extract_all_pdf(path: &Path, target_dir: &Path) -> Result<Vec<PathBuf>> {
    let output_prefix = target_dir.join("page");
    let status = Command::new("pdftoppm")
        .args(["-jpeg", path.to_str().unwrap_or(""), output_prefix.to_str().unwrap_or("")])
        .status()?;

    if !status.success() {
        bail!("pdftoppm falló extrayendo {}", path.display());
    }

    collect_and_sort_images(target_dir)
}

// ---------------------------------------------------------------------------
// API de portada (usada por el escáner de biblioteca)
// ---------------------------------------------------------------------------

pub fn extract_cover(path: &str) -> Result<Vec<u8>> {
    let p = Path::new(path);
    match ComicFormat::from_path(p) {
        ComicFormat::Cbz => extract_cover_zip(p),
        ComicFormat::Cbr => extract_cover_rar(p),
        ComicFormat::Cb7 => extract_cover_7z(p),
        ComicFormat::Pdf => extract_cover_pdf(p),
        ComicFormat::Unknown => bail!("Formato no soportado: {}", path),
    }
}

fn extract_cover_zip(path: &Path) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("No se pudo abrir: {}", path.display()))?;

    let mut archive = zip::ZipArchive::new(file)
        .with_context(|| format!("ZIP inválido: {}", path.display()))?;

    let mut image_names: Vec<String> = (0..archive.len())
        .filter_map(|i| {
            archive.by_index(i).ok().and_then(|f| {
                let name = f.name().to_lowercase();
                if is_image_name(&name) { Some(f.name().to_string()) } else { None }
            })
        })
        .collect();

    if image_names.is_empty() {
        bail!("No se encontraron imágenes en: {}", path.display());
    }

    image_names.sort();
    let first = image_names[0].clone();

    let mut entry = archive
        .by_name(&first)
        .with_context(|| format!("No se pudo leer '{}' del ZIP", first))?;

    let mut bytes = Vec::new();
    entry.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn extract_cover_rar(path: &Path) -> Result<Vec<u8>> {
    let list_output = Command::new("unrar")
        .args(["lb", path.to_str().unwrap_or("")])
        .output()
        .context("No se pudo ejecutar 'unrar'. Verifica que está instalado.")?;

    let listing = String::from_utf8_lossy(&list_output.stdout);
    let mut image_names: Vec<&str> = listing
        .lines()
        .filter(|l| is_image_name(&l.to_lowercase()))
        .collect();

    if image_names.is_empty() {
        bail!("No se encontraron imágenes en: {}", path.display());
    }

    image_names.sort();
    let first = image_names[0];

    let tmp_dir = tempfile_dir()?;
    let status = Command::new("unrar")
        .args(["e", "-y", "-o+", path.to_str().unwrap_or(""), first, &tmp_dir])
        .status()
        .context("Error ejecutando unrar")?;

    if !status.success() {
        bail!("unrar falló extrayendo '{}' de {}", first, path.display());
    }

    let file_name = Path::new(first).file_name().unwrap_or_default().to_str().unwrap_or("");
    let extracted = Path::new(&tmp_dir).join(file_name);

    let bytes = std::fs::read(&extracted)
        .with_context(|| format!("No se pudo leer archivo extraído: {}", extracted.display()))?;

    let _ = std::fs::remove_file(&extracted);
    Ok(bytes)
}

fn extract_cover_7z(path: &Path) -> Result<Vec<u8>> {
    use sevenz_rust::Archive;
    use std::io::BufReader;

    let file = std::fs::File::open(path)
        .with_context(|| format!("No se pudo abrir: {}", path.display()))?;

    let mut reader = BufReader::new(file);
    let archive = Archive::read(&mut reader, path.metadata()?.len(), &[])
        .with_context(|| format!("7Z inválido: {}", path.display()))?;

    let mut image_entries: Vec<(String, usize)> = archive
        .files
        .iter()
        .enumerate()
        .filter(|(_, f)| !f.is_directory() && is_image_name(&f.name().to_lowercase()))
        .map(|(i, f)| (f.name().to_string(), i))
        .collect();

    if image_entries.is_empty() {
        bail!("No se encontraron imágenes en: {}", path.display());
    }

    image_entries.sort_by(|a, b| a.0.cmp(&b.0));
    let (_, entry_idx) = &image_entries[0];

    let tmp_dir = tempfile_dir()?;
    sevenz_rust::decompress_file(path, &tmp_dir)
        .map_err(|e| anyhow::anyhow!("Error descomprimiendo 7Z: {}", e))?;

    let file_name = Path::new(&image_entries[0].0)
        .file_name()
        .unwrap_or_default()
        .to_str()
        .unwrap_or("");

    let extracted = find_file_in_dir(&tmp_dir, file_name)?;
    let bytes = std::fs::read(&extracted)?;

    let _ = std::fs::remove_file(&extracted);
    let _ = entry_idx;
    Ok(bytes)
}

fn extract_cover_pdf(path: &Path) -> Result<Vec<u8>> {
    let tmp_dir = tempfile_dir()?;
    let output_prefix = Path::new(&tmp_dir).join("pdf_cover");

    let status = Command::new("pdftoppm")
        .args([
            "-f", "1",
            "-l", "1",
            "-jpeg",
            "-singlefile",
            path.to_str().unwrap_or(""),
            output_prefix.to_str().unwrap_or(""),
        ])
        .status()
        .context("No se pudo ejecutar 'pdftoppm'. Verifica que 'poppler-utils' está instalado.")?;

    if !status.success() {
        bail!("pdftoppm falló extrayendo la portada de {}", path.display());
    }

    let extracted = Path::new(&tmp_dir).join("pdf_cover.jpg");
    let bytes = std::fs::read(&extracted)
        .with_context(|| format!("No se pudo leer la portada PDF extraída: {}", extracted.display()))?;

    let _ = std::fs::remove_file(&extracted);
    Ok(bytes)
}

// ---------------------------------------------------------------------------
// Utilidades compartidas
// ---------------------------------------------------------------------------

fn collect_and_sort_images(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut images = Vec::new();
    for entry in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if is_image_name(&name) {
                images.push(entry.into_path());
            }
        }
    }

    images.sort_by(|a, b| {
        let a_str = a.file_name().unwrap_or_default().to_string_lossy();
        let b_str = b.file_name().unwrap_or_default().to_string_lossy();
        natural_sort_compare(&a_str, &b_str)
    });

    Ok(images)
}

fn natural_sort_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let a_parts = split_natural(a);
    let b_parts = split_natural(b);

    for (a_p, b_p) in a_parts.iter().zip(b_parts.iter()) {
        match (a_p.parse::<u64>(), b_p.parse::<u64>()) {
            (Ok(a_n), Ok(b_n)) => {
                if a_n != b_n { return a_n.cmp(&b_n); }
            }
            _ => {
                let cmp = a_p.to_lowercase().cmp(&b_p.to_lowercase());
                if cmp != std::cmp::Ordering::Equal { return cmp; }
            }
        }
    }
    a_parts.len().cmp(&b_parts.len())
}

fn split_natural(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut is_digit = false;

    for c in s.chars() {
        if c.is_ascii_digit() != is_digit {
            if !current.is_empty() {
                parts.push(current);
                current = String::new();
            }
            is_digit = c.is_ascii_digit();
        }
        current.push(c);
    }
    if !current.is_empty() { parts.push(current); }
    parts
}

fn is_image_name(name: &str) -> bool {
    if name.contains("__MACOSX") || name.starts_with('.') {
        return false;
    }
    Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| IMAGE_EXTENSIONS.contains(&e))
        .unwrap_or(false)
}

fn tempfile_dir() -> Result<String> {
    let dir = std::env::temp_dir().join("babelcomics_extract");
    std::fs::create_dir_all(&dir)?;
    Ok(dir.to_string_lossy().to_string())
}

fn find_file_in_dir(dir: &str, filename: &str) -> Result<PathBuf> {
    for entry in walkdir::WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        if entry.file_type().is_file() {
            if let Some(name) = entry.file_name().to_str() {
                if name == filename {
                    return Ok(entry.into_path());
                }
            }
        }
    }
    bail!("No se encontró '{}' en directorio temporal", filename)
}
