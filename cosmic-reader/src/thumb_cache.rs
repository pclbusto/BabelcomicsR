//! Caché de miniaturas guardada dentro del propio cómic (`babel-thb`).
//!
//! Formato (enteros u32 little-endian):
//!
//! ```text
//! "BABELTHB" | versión | ancho máx. | alto máx. | cantidad
//! repetido: largo del nombre | nombre UTF-8 | largo del JPEG | JPEG
//! ```
//!
//! Cada miniatura se asocia al nombre de la página dentro del archivo, así una
//! caché que no corresponde exactamente a las páginas actuales se descarta.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub const ENTRY_NAME: &str = "babel-thb";
const MAGIC: &[u8; 8] = b"BABELTHB";
const VERSION: u32 = 1;

pub struct CachedThumbnail {
    pub page_name: String,
    pub jpeg: Vec<u8>,
}

pub fn serialize(width: u32, height: u32, thumbnails: &[(&str, &[u8])]) -> Vec<u8> {
    let payload: usize = thumbnails
        .iter()
        .map(|(name, jpeg)| 8 + name.len() + jpeg.len())
        .sum();
    let mut out = Vec::with_capacity(MAGIC.len() + 16 + payload);
    out.extend_from_slice(MAGIC);
    for value in [VERSION, width, height, thumbnails.len() as u32] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    for (name, jpeg) in thumbnails {
        out.extend_from_slice(&(name.len() as u32).to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&(jpeg.len() as u32).to_le_bytes());
        out.extend_from_slice(jpeg);
    }
    out
}

/// Devuelve `None` si la caché está dañada, es de otra versión o se generó con
/// otro tamaño de miniatura.
pub fn parse(bytes: &[u8], width: u32, height: u32) -> Option<Vec<CachedThumbnail>> {
    let mut cursor = bytes.strip_prefix(MAGIC.as_slice())?;
    let read_u32 = |cursor: &mut &[u8]| -> Option<u32> {
        let (value, rest) = cursor.split_first_chunk::<4>()?;
        *cursor = rest;
        Some(u32::from_le_bytes(*value))
    };
    if read_u32(&mut cursor)? != VERSION
        || read_u32(&mut cursor)? != width
        || read_u32(&mut cursor)? != height
    {
        return None;
    }
    let count = read_u32(&mut cursor)? as usize;
    // Cada entrada ocupa al menos 8 bytes: evita reservar memoria absurda.
    let mut thumbnails = Vec::with_capacity(count.min(cursor.len() / 8));
    for _ in 0..count {
        let name_len = read_u32(&mut cursor)? as usize;
        let (name, rest) = cursor.split_at_checked(name_len)?;
        cursor = rest;
        let page_name = std::str::from_utf8(name).ok()?.to_string();
        let jpeg_len = read_u32(&mut cursor)? as usize;
        let (jpeg, rest) = cursor.split_at_checked(jpeg_len)?;
        cursor = rest;
        thumbnails.push(CachedThumbnail {
            page_name,
            jpeg: jpeg.to_vec(),
        });
    }
    cursor.is_empty().then_some(thumbnails)
}

pub fn read_from_zip(archive: &mut ZipArchive<File>) -> Option<Vec<u8>> {
    let mut entry = archive.by_name(ENTRY_NAME).ok()?;
    let mut bytes = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}

pub fn read_from_rar(path: &Path) -> Option<Vec<u8>> {
    let output = Command::new("unrar")
        .args(["p", "-inul"])
        .arg(path)
        .arg(ENTRY_NAME)
        .output()
        .ok()?;
    (output.status.success() && !output.stdout.is_empty()).then_some(output.stdout)
}

fn is_rar(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension.to_ascii_lowercase().as_str(), "cbr" | "rar"))
}

/// Guarda la caché dentro del cómic, conservando su fecha de modificación y
/// sus permisos.
pub fn write_into_archive(path: &Path, data: &[u8]) -> Result<(), String> {
    let metadata = fs::metadata(path)
        .map_err(|error| format!("No se pudo leer {}: {error}", path.display()))?;
    if is_rar(path) {
        write_rar(path, data)?;
    } else {
        write_zip(path, data, &metadata)?;
    }
    if let Ok(modified) = metadata.modified() {
        let _ = File::options()
            .write(true)
            .open(path)
            .and_then(|file| file.set_modified(modified));
    }
    Ok(())
}

/// Reescribe el ZIP en un archivo temporal y lo renombra sobre el original, así
/// un fallo a mitad de camino nunca deja el cómic dañado. Las entradas se copian
/// sin recomprimir.
fn write_zip(path: &Path, data: &[u8], metadata: &fs::Metadata) -> Result<(), String> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let temporary = path.with_file_name(format!(".{file_name}.babel-thb.tmp"));

    let result = (|| -> Result<(), String> {
        let source = File::open(path)
            .map_err(|error| format!("No se pudo abrir {}: {error}", path.display()))?;
        let mut archive =
            ZipArchive::new(source).map_err(|error| format!("ZIP no válido: {error}"))?;
        let target = File::create(&temporary)
            .map_err(|error| format!("No se pudo crear {}: {error}", temporary.display()))?;
        let mut writer = ZipWriter::new(target);
        writer.set_raw_comment(archive.comment().into());
        for index in 0..archive.len() {
            let entry = archive
                .by_index_raw(index)
                .map_err(|error| format!("No se pudo leer la entrada {index}: {error}"))?;
            if entry.name() == ENTRY_NAME {
                continue;
            }
            writer
                .raw_copy_file(entry)
                .map_err(|error| format!("No se pudo copiar la entrada {index}: {error}"))?;
        }
        // Los JPEG ya están comprimidos.
        writer
            .start_file(
                ENTRY_NAME,
                SimpleFileOptions::default().compression_method(CompressionMethod::Stored),
            )
            .map_err(|error| format!("No se pudo agregar {ENTRY_NAME}: {error}"))?;
        writer
            .write_all(data)
            .map_err(|error| format!("No se pudo escribir {ENTRY_NAME}: {error}"))?;
        let target = writer
            .finish()
            .map_err(|error| format!("No se pudo cerrar el ZIP: {error}"))?;
        target
            .sync_all()
            .map_err(|error| format!("No se pudo sincronizar el ZIP: {error}"))?;
        drop(target);
        let _ = fs::set_permissions(&temporary, metadata.permissions());
        fs::rename(&temporary, path)
            .map_err(|error| format!("No se pudo reemplazar {}: {error}", path.display()))
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

/// RAR solo se puede modificar con el programa `rar` (no libre).
fn write_rar(path: &Path, data: &[u8]) -> Result<(), String> {
    let directory = std::env::temp_dir().join(format!("babel-thb-{}", std::process::id()));
    fs::create_dir_all(&directory)
        .map_err(|error| format!("No se pudo crear {}: {error}", directory.display()))?;
    let entry_path: PathBuf = directory.join(ENTRY_NAME);
    let result = fs::write(&entry_path, data)
        .map_err(|error| format!("No se pudo preparar {ENTRY_NAME}: {error}"))
        .and_then(|()| {
            let status = Command::new("rar")
                .args(["a", "-ep", "-inul", "-o+"])
                .arg(path)
                .arg(&entry_path)
                .status()
                .map_err(|error| {
                    format!("No se pudo ejecutar rar (necesario para modificar CBR): {error}")
                })?;
            if status.success() {
                Ok(())
            } else {
                Err(format!("rar terminó con {status}"))
            }
        });
    let _ = fs::remove_dir_all(&directory);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let data = serialize(180, 220, &[("01.jpg", b"abc"), ("02.jpg", b"")]);
        let parsed = parse(&data, 180, 220).expect("caché válida");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].page_name, "01.jpg");
        assert_eq!(parsed[0].jpeg, b"abc");
        assert_eq!(parsed[1].page_name, "02.jpg");
    }

    #[test]
    fn rejects_other_sizes_and_truncated_data() {
        let data = serialize(180, 220, &[("01.jpg", b"abc")]);
        assert!(parse(&data, 100, 220).is_none());
        assert!(parse(&data[..data.len() - 1], 180, 220).is_none());
        assert!(parse(b"basura", 180, 220).is_none());
    }

    #[test]
    fn writes_cache_into_zip_keeping_pages() {
        let directory = std::env::temp_dir().join(format!("babel-thb-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.cbz");
        {
            let mut writer = ZipWriter::new(File::create(&path).unwrap());
            writer
                .start_file("01.jpg", SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"page").unwrap();
            writer.finish().unwrap();
        }

        write_into_archive(&path, b"first").unwrap();
        write_into_archive(&path, b"second").unwrap();

        let mut archive = ZipArchive::new(File::open(&path).unwrap()).unwrap();
        assert_eq!(archive.len(), 2, "la caché anterior se reemplaza");
        assert_eq!(read_from_zip(&mut archive).unwrap(), b"second");
        let mut page = Vec::new();
        archive
            .by_name("01.jpg")
            .unwrap()
            .read_to_end(&mut page)
            .unwrap();
        assert_eq!(page, b"page");
        fs::remove_dir_all(&directory).unwrap();
    }
}
