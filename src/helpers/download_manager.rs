use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use tokio::sync::{mpsc, broadcast};
use serde_json::Value;
use sqlx::SqlitePool;
use crate::helpers::comicvine_client::ComicVineClient;

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
    #[allow(dead_code)]
    pool: SqlitePool,
    active_downloads: Arc<Mutex<HashMap<String, DownloadInfo>>>,
    tx_queue: mpsc::UnboundedSender<String>,
    tx_events: broadcast::Sender<DownloadEvent>,
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
            
            // Iniciar el worker loop
            let dm_clone = dm.clone();
            tokio::spawn(async move {
                dm_clone.worker_loop(rx_queue).await;
            });
            
            *instance = Some(dm);
        }
        instance.as_ref().unwrap().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<DownloadEvent> {
        self.tx_events.subscribe()
    }

    pub fn add_download(&self, volume_data: Value, _client: ComicVineClient, _download_covers: bool) {
        let cv_id = volume_data["id"].as_u64().unwrap_or(0).to_string();
        if cv_id == "0" { return; }

        let mut active = self.active_downloads.lock().unwrap();
        if active.contains_key(&cv_id) { return; }

        let info = DownloadInfo {
            volume_cv_id: cv_id.clone(),
            title: volume_data["name"].as_str().unwrap_or("Desconocido").to_string(),
            total_issues: volume_data["count_of_issues"].as_i64().unwrap_or(0),
            progress: 0.0,
            message: "En cola...".to_string(),
            status: DownloadStatus::Queued,
        };

        active.insert(cv_id.clone(), info);
        let _ = self.tx_events.send(DownloadEvent::Added(cv_id.clone()));
        let _ = self.tx_queue.send(cv_id);
    }

    async fn worker_loop(self: Arc<Self>, mut rx: mpsc::UnboundedReceiver<String>) {
        while let Some(cv_id) = rx.recv().await {
            let _exists = {
                let active = self.active_downloads.lock().unwrap();
                active.contains_key(&cv_id)
            };

            if _exists {
                self.update_status(&cv_id, DownloadStatus::Downloading, 0.0, "Iniciando...");
                
                match self.process_download(&cv_id).await {
                    Ok(_) => {
                        self.update_status(&cv_id, DownloadStatus::Completed, 1.0, "¡Completado!");
                        let _ = self.tx_events.send(DownloadEvent::Completed(cv_id));
                    }
                    Err(e) => {
                        let err_msg = e.to_string();
                        self.update_status(&cv_id, DownloadStatus::Error(err_msg.clone()), 0.0, &format!("Error: {}", err_msg));
                        let _ = self.tx_events.send(DownloadEvent::Error(cv_id, err_msg));
                    }
                }
            }
        }
    }

    fn update_status(&self, cv_id: &str, status: DownloadStatus, progress: f64, msg: &str) {
        let mut active = self.active_downloads.lock().unwrap();
        if let Some(info) = active.get_mut(cv_id) {
            info.status = status;
            info.progress = progress;
            info.message = msg.to_string();
            let _ = self.tx_events.send(DownloadEvent::Progress(cv_id.to_string(), progress, msg.to_string()));
        }
    }

    async fn process_download(&self, cv_id: &str) -> anyhow::Result<()> {
        for i in 1..=10 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            self.update_status(cv_id, DownloadStatus::Downloading, i as f64 / 10.0, &format!("Descargando batch {}/10...", i));
        }
        Ok(())
    }
}
