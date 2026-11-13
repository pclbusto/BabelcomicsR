use crate::helpers::comicvine_client::ComicVineClient;
use crate::helpers::paths::comicbook_info_thumbnail_path;
use crate::models::{NewComicbookInfo, NewComicbookInfoCover, NewPublisher, NewVolume};
use crate::repositories::SetupRepository;
use crate::repositories::{ComicbookInfoRepository, PublisherRepository, VolumeRepository};
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast, mpsc};

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadStatus {
    Queued,
    Downloading,
    Completed,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct DownloadInfo {
    pub volume_cv_id: String,
    pub title: String,
    pub total_issues: i64,
    pub progress: f64,
    pub message: String,
    pub status: DownloadStatus,
    pub download_covers: bool,
    pub background_job_id: String,
}

#[derive(Clone)]
pub enum DownloadEvent {
    Added(String),
    Progress(String, f64, String),
    Completed(String),
    Error(String, String),
    #[allow(dead_code)]
    Removed(String),
}

pub struct DownloadManager {
    pool: SqlitePool,
    active_downloads: Arc<Mutex<HashMap<String, DownloadInfo>>>,
    tx_queue: mpsc::UnboundedSender<String>,
    tx_events: broadcast::Sender<DownloadEvent>,
}

struct DownloadResult {
    volume_id: i64,
    downloaded_issues: usize,
    total_issues: usize,
}

lazy_static::lazy_static! {
    static ref INSTANCE: Arc<Mutex<Option<Arc<DownloadManager>>>> = Arc::new(Mutex::new(None));
}

impl DownloadManager {
    pub fn get_instance(pool: SqlitePool) -> Arc<Self> {
        let mut instance = INSTANCE.lock().unwrap();
        if instance.is_none() {
            let (tx_queue, rx_queue) = mpsc::unbounded_channel::<String>();
            let (tx_events, _) = broadcast::channel(100);

            let dm = Arc::new(DownloadManager {
                pool: pool.clone(),
                active_downloads: Arc::new(Mutex::new(HashMap::new())),
                tx_queue,
                tx_events,
            });

            let dm_clone = dm.clone();
            std::thread::Builder::new()
                .name("babelcomics-downloads".to_string())
                .spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("no se pudo iniciar el runtime de descargas");
                    runtime.block_on(dm_clone.worker_loop(rx_queue));
                })
                .expect("no se pudo iniciar el hilo de descargas");

            *instance = Some(dm);
        }
        instance.as_ref().unwrap().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DownloadEvent> {
        self.tx_events.subscribe()
    }

    pub fn get_state(&self) -> Arc<Mutex<HashMap<String, DownloadInfo>>> {
        self.active_downloads.clone()
    }

    pub fn add_download(
        &self,
        cv_id: i64,
        name: &str,
        count_of_issues: i64,
        download_covers: bool,
    ) -> bool {
        if cv_id == 0 {
            return false;
        }
        let cv_id_str = cv_id.to_string();

        let mut active = self.active_downloads.lock().unwrap();
        if let Some(previous) = active.get(&cv_id_str) {
            if matches!(
                previous.status,
                DownloadStatus::Queued | DownloadStatus::Downloading
            ) {
                return false;
            }
            active.remove(&cv_id_str);
        }

        let background_job_id = crate::helpers::background_jobs::start_job(
            name,
            "En cola...",
            crate::helpers::background_jobs::JobKind::Download,
        );
        let info = DownloadInfo {
            volume_cv_id: cv_id_str.clone(),
            title: name.to_string(),
            total_issues: count_of_issues,
            progress: 0.0,
            message: "En cola...".to_string(),
            status: DownloadStatus::Queued,
            download_covers,
            background_job_id,
        };

        active.insert(cv_id_str.clone(), info);
        let _ = self.tx_events.send(DownloadEvent::Added(cv_id_str.clone()));
        let _ = self.tx_queue.send(cv_id_str);
        true
    }

    async fn worker_loop(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<String>) {
        while let Some(cv_id) = rx.recv().await {
            let exists = {
                let active = self.active_downloads.lock().unwrap();
                active.contains_key(&cv_id)
            };

            if exists {
                self.update_status(&cv_id, DownloadStatus::Downloading, 0.0, "Iniciando...");

                match self.process_download(&cv_id).await {
                    Ok(result) => {
                        let completed_msg = format!(
                            "Completado: {}/{} números",
                            result.downloaded_issues, result.total_issues
                        );
                        self.update_status(&cv_id, DownloadStatus::Completed, 1.0, &completed_msg);
                        let _ = self.tx_events.send(DownloadEvent::Completed(cv_id.clone()));
                        tracing::debug!(
                            volume_id = result.volume_id,
                            "CLIP post-descarga omitido; ejecutar indexación manual bajo demanda"
                        );
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        self.update_status(
                            &cv_id,
                            DownloadStatus::Error(err_msg.clone()),
                            0.0,
                            &format!("Error: {}", err_msg),
                        );
                        let _ = self.tx_events.send(DownloadEvent::Error(cv_id, err_msg));
                    }
                }
            }
        }
    }

    fn update_status(&self, cv_id: &str, status: DownloadStatus, progress: f64, msg: &str) {
        let bg_info = {
            let mut active = self.active_downloads.lock().unwrap();
            active.get_mut(cv_id).map(|info| {
                info.status = status.clone();
                info.progress = progress;
                info.message = msg.to_string();
                let _ = self.tx_events.send(DownloadEvent::Progress(
                    cv_id.to_string(),
                    progress,
                    msg.to_string(),
                ));
                (info.background_job_id.clone(), status)
            })
        };
        if let Some((job_id, status)) = bg_info {
            match status {
                DownloadStatus::Completed => {
                    crate::helpers::background_jobs::finish_job(&job_id, msg, false);
                }
                DownloadStatus::Error(_) => {
                    crate::helpers::background_jobs::finish_job(&job_id, msg, true);
                }
                _ => {
                    crate::helpers::background_jobs::update_job(&job_id, msg, progress);
                }
            }
        }
    }

    async fn process_download(&self, cv_id: &str) -> anyhow::Result<DownloadResult> {
        // 1. Obtener API key y crear cliente
        let setup = SetupRepository::new(&self.pool).get().await?;
        let api_key = setup
            .api_key_encrypted
            .as_deref()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow::anyhow!("No hay API key configurada"))?
            .to_string();

        let base_url = setup.api_url.clone();
        let client = ComicVineClient::new(api_key, base_url)?;

        // 2. Obtener detalles del volumen
        self.update_status(
            cv_id,
            DownloadStatus::Downloading,
            0.02,
            "Obteniendo detalles del volumen...",
        );
        let vol_details = client
            .get_volume_details(cv_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("No se pudo obtener el volumen de ComicVine"))?;

        let vol_name = vol_details["name"]
            .as_str()
            .unwrap_or("Desconocido")
            .to_string();
        let vol_cv_id = vol_details["id"].as_i64().unwrap_or(0);
        let vol_deck = strip_html(vol_details["deck"].as_str().unwrap_or(""));
        let vol_desc = strip_html(vol_details["description"].as_str().unwrap_or(""));
        let vol_url = vol_details["site_detail_url"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let vol_year = vol_details["start_year"]
            .as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let vol_count = vol_details["count_of_issues"].as_i64().unwrap_or(0);
        let vol_image = vol_details["image"]["medium_url"]
            .as_str()
            .unwrap_or("")
            .to_string();

        let http_client = reqwest::Client::builder()
            .user_agent("BabelcomicsR/1.0 (Rust)")
            .build()?;

        // 3. Upsert editorial
        self.update_status(
            cv_id,
            DownloadStatus::Downloading,
            0.05,
            "Guardando editorial...",
        );
        let publisher_id = self.upsert_publisher(&vol_details).await?;

        // 4. Upsert volumen
        self.update_status(
            cv_id,
            DownloadStatus::Downloading,
            0.08,
            "Guardando serie...",
        );
        let volume_id = self
            .upsert_volume(
                vol_cv_id,
                &vol_name,
                &vol_deck,
                &vol_desc,
                &vol_url,
                &vol_image,
                publisher_id,
                vol_year,
                vol_count,
            )
            .await?;

        // Descargar la portada del volumen si no tenemos una ya (como fallback)
        if !vol_image.is_empty() {
            let vt_path = crate::helpers::paths::volume_thumbnail_path(volume_id);
            if !vt_path.exists() {
                if let Ok(resp) = http_client.get(&vol_image).send().await {
                    if let Ok(bytes) = resp.bytes().await {
                        if let Some(parent) = vt_path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        let _ = std::fs::write(&vt_path, &bytes);
                    }
                }
            }
        }

        // 5. Obtener todos los issues
        self.update_status(
            cv_id,
            DownloadStatus::Downloading,
            0.10,
            "Obteniendo lista de números...",
        );
        let issues = client.get_volume_issues(cv_id).await;
        let total_issues = issues.len();
        let total_for_progress = total_issues.max(1);

        // Cruce: Si la cantidad de issues obtenidos difiere de vol_count, actualizar el volumen
        if total_issues as i64 != vol_count {
            let v_repo = VolumeRepository::new(&self.pool);
            if let Ok(Some(mut vol)) = v_repo.get_by_id(volume_id).await {
                vol.cantidad_numeros = total_issues as i64;
                let _ = v_repo.update(&vol).await;
            }
        }

        // 6. Procesar cada issue
        let download_covers = {
            let active = self.active_downloads.lock().unwrap();
            active
                .get(cv_id)
                .map(|i| i.download_covers)
                .unwrap_or(false)
        };

        let mut downloaded_issues = 0usize;
        for (i, issue) in issues.iter().enumerate() {
            let progress = 0.10 + 0.90 * (i as f64 / total_for_progress as f64);
            let issue_num = issue["issue_number"].as_str().unwrap_or("?");
            let msg = format!("Número {} ({}/{})", issue_num, i + 1, total_issues);
            self.update_status(cv_id, DownloadStatus::Downloading, progress, &msg);

            let issue_cv_id = match issue["id"].as_i64() {
                Some(id) => id,
                None => continue,
            };

            let repo = ComicbookInfoRepository::new(&self.pool);

            let titulo = issue["name"].as_str().unwrap_or("").to_string();
            let numero = issue["issue_number"].as_str().map(|s| s.to_string());
            let resumen = issue["description"].as_str().map(strip_html);
            let url_api = issue_detail_url(issue);
            let titulo = if titulo.is_empty() {
                format!("{} #{}", vol_name, numero.as_deref().unwrap_or("?"))
            } else {
                titulo
            };

            // Actualizar los metadatos si el número ya existía; insertar si es nuevo.
            let info_id = if let Some(mut existing) = repo.get_by_comicvine_id(issue_cv_id).await? {
                existing.titulo = titulo;
                existing.id_volume = Some(volume_id);
                existing.numero = numero;
                existing.resumen = resumen;
                existing.url_api_detalle = url_api;
                existing.fue_actualizado_api = true;
                repo.update(&existing).await?;
                existing.id_comicbook_info
            } else {
                repo.insert(&NewComicbookInfo {
                    titulo,
                    id_volume: Some(volume_id),
                    numero,
                    resumen,
                    calificacion: None,
                    id_comicvine: Some(issue_cv_id),
                    url_api_detalle: url_api,
                })
                .await?
            };

            // ComicVine expone la portada principal en `image` y las variantes en
            // `associated_images`. Guardamos una fila por imagen para que la UI y
            // el índice CLIP puedan trabajar con todas ellas.
            let cover_urls = issue_cover_urls(issue);
            let existing_covers = repo.get_covers(info_id).await?;
            for (cover_index, url) in cover_urls.into_iter().enumerate() {
                if let Some(existing) = existing_covers
                    .iter()
                    .find(|cover| cover.url_original == url)
                {
                    if download_covers
                        && existing.ruta_local.is_none()
                        && let Ok(local_path) = self
                            .download_cover(
                                &http_client,
                                &url,
                                &vol_name,
                                vol_cv_id,
                                issue_cv_id,
                                cover_index,
                            )
                            .await
                    {
                        let _ = repo.set_cover_local_path(existing.id, &local_path).await;
                    }
                    continue;
                }

                let local_path = if download_covers {
                    self.download_cover(
                        &http_client,
                        &url,
                        &vol_name,
                        vol_cv_id,
                        issue_cv_id,
                        cover_index,
                    )
                    .await
                    .ok()
                } else {
                    None
                };
                repo.insert_cover(&NewComicbookInfoCover {
                    id_comicbook_info: info_id,
                    url_original: url,
                    ruta_local: local_path,
                })
                .await?;
            }

            downloaded_issues += 1;
        }

        Ok(DownloadResult {
            volume_id,
            downloaded_issues,
            total_issues,
        })
    }

    async fn upsert_publisher(&self, vol_details: &Value) -> anyhow::Result<i64> {
        let pub_data = &vol_details["publisher"];
        let pub_cv_id = pub_data["id"].as_i64().unwrap_or(0);

        let repo = PublisherRepository::new(&self.pool);

        if pub_cv_id > 0 {
            if let Some(mut existing) = repo.get_by_comicvine_id(pub_cv_id).await? {
                existing.nombre = pub_data["name"]
                    .as_str()
                    .unwrap_or(&existing.nombre)
                    .to_string();
                repo.update(&existing).await?;
                return Ok(existing.id_publisher);
            }
        }

        let nombre = pub_data["name"]
            .as_str()
            .unwrap_or("Desconocido")
            .to_string();
        let id = repo
            .insert(&NewPublisher {
                nombre,
                descripcion: None,
                id_comicvine: if pub_cv_id > 0 { Some(pub_cv_id) } else { None },
                image_url: None,
            })
            .await?;

        Ok(id)
    }

    async fn upsert_volume(
        &self,
        cv_id: i64,
        nombre: &str,
        deck: &str,
        descripcion: &str,
        url: &str,
        image_url: &str,
        id_publisher: i64,
        anio_inicio: i64,
        cantidad_numeros: i64,
    ) -> anyhow::Result<i64> {
        let repo = VolumeRepository::new(&self.pool);

        if cv_id > 0 {
            if let Some(mut existing) = repo.get_by_comicvine_id(cv_id).await? {
                existing.nombre = nombre.to_string();
                existing.deck = deck.to_string();
                existing.descripcion = descripcion.to_string();
                existing.url = url.to_string();
                existing.image_url = image_url.to_string();
                existing.id_publisher = id_publisher;
                existing.anio_inicio = anio_inicio;
                existing.cantidad_numeros = cantidad_numeros;
                repo.update(&existing).await?;
                return Ok(existing.id_volume);
            }
        }

        let id = repo
            .insert(&NewVolume {
                nombre: nombre.to_string(),
                deck: deck.to_string(),
                descripcion: descripcion.to_string(),
                url: url.to_string(),
                image_url: image_url.to_string(),
                id_publisher,
                anio_inicio,
                cantidad_numeros,
                id_comicvine: if cv_id > 0 { Some(cv_id) } else { None },
            })
            .await?;

        Ok(id)
    }

    async fn download_cover(
        &self,
        client: &reqwest::Client,
        url: &str,
        vol_name: &str,
        vol_cv_id: i64,
        issue_cv_id: i64,
        cover_index: usize,
    ) -> anyhow::Result<String> {
        let bytes = client.get(url).send().await?.bytes().await?;
        let ext = url
            .split('?')
            .next()
            .and_then(|path| path.rsplit('.').next())
            .filter(|ext| ext.len() <= 5)
            .unwrap_or("jpg");
        let filename = format!("issue_{}_{}.{}", issue_cv_id, cover_index, ext);
        let path = comicbook_info_thumbnail_path(vol_name, vol_cv_id, &filename);

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &bytes)?;

        Ok(path.to_string_lossy().to_string())
    }
}

fn issue_cover_urls(issue: &Value) -> Vec<String> {
    let mut urls = Vec::new();
    if let Some(url) = issue["image"]["medium_url"]
        .as_str()
        .filter(|url| !url.is_empty())
    {
        urls.push(url.to_string());
    }
    if let Some(images) = issue["associated_images"].as_array() {
        for image in images {
            if let Some(url) = image["original_url"].as_str().filter(|url| !url.is_empty())
                && !urls.iter().any(|existing| {
                    existing == url || cover_asset_key(existing) == cover_asset_key(url)
                })
            {
                urls.push(url.to_string());
            }
        }
    }
    urls
}

fn issue_detail_url(issue: &Value) -> Option<String> {
    issue["site_detail_url"]
        .as_str()
        .or_else(|| issue["api_detail_url"].as_str())
        .filter(|url| !url.is_empty())
        .map(str::to_string)
}

fn cover_asset_key(url: &str) -> &str {
    url.split('?')
        .next()
        .unwrap_or(url)
        .rsplit('/')
        .next()
        .unwrap_or(url)
}

fn strip_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(c),
            _ => {}
        }
    }
    // Decode common HTML entities
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn collects_primary_and_alternate_issue_covers_without_duplicates() {
        let issue = json!({
            "image": { "medium_url": "https://img.test/primary.jpg" },
            "associated_images": [
                { "original_url": "https://img.test/variant-a.jpg" },
                { "original_url": "https://img.test/variant-b.jpg" },
                { "original_url": "https://img.test/variant-a.jpg" },
                { "original_url": "https://img.test/original/primary.jpg" },
                { "original_url": "" }
            ]
        });

        assert_eq!(
            issue_cover_urls(&issue),
            vec![
                "https://img.test/primary.jpg",
                "https://img.test/variant-a.jpg",
                "https://img.test/variant-b.jpg",
            ]
        );
    }

    #[test]
    fn prefers_the_public_comicvine_issue_page() {
        let issue = json!({
            "site_detail_url": "https://comicvine.gamespot.com/example/4000-42/",
            "api_detail_url": "https://comicvine.gamespot.com/api/issue/4000-42/"
        });

        assert_eq!(
            issue_detail_url(&issue).as_deref(),
            Some("https://comicvine.gamespot.com/example/4000-42/")
        );
    }

    #[test]
    fn initializes_outside_a_tokio_runtime() {
        let bootstrap_runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let pool = bootstrap_runtime.block_on(async {
            SqlitePoolOptions::new()
                .max_connections(1)
                .connect("sqlite::memory:")
                .await
                .unwrap()
        });
        assert!(tokio::runtime::Handle::try_current().is_err());

        let manager = DownloadManager::get_instance(pool);

        assert!(manager.get_state().lock().unwrap().is_empty());
    }
}

/// Descarga los bytes de una imagen desde una URL.
/// Devuelve `None` si la URL está vacía o la descarga falla.
pub async fn fetch_image_bytes(url: &str) -> Option<Vec<u8>> {
    if url.is_empty() {
        return None;
    }
    let bytes = reqwest::get(url).await.ok()?.bytes().await.ok()?;
    Some(bytes.to_vec())
}
