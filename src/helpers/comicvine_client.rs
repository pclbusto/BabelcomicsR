use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use serde_json::Value;
use tokio::sync::Mutex;

const BASE_URL: &str = "https://comicvine.gamespot.com/api/";
const API_RESULTS_LIMIT: usize = 100;
const MAX_CONCURRENT: usize = 5;
const REQUEST_INTERVAL: Duration = Duration::from_millis(500);

fn resource_prefix(resource_type: &str) -> Option<&'static str> {
    match resource_type {
        "volume" => Some("4050-"),
        "publisher" => Some("4040-"),
        "character" => Some("4005-"),
        "issue" => Some("4000-"),
        _ => None,
    }
}

fn format_resource_id(id: &str, resource_type: &str) -> String {
    if let Some(prefix) = resource_prefix(resource_type) {
        if !id.starts_with(prefix) {
            return format!("{}{}", prefix, id);
        }
    }
    id.to_string()
}

// Los parámetros se pasan como pares (clave, valor) en strings propios para
// evitar problemas de lifetimes con referencias a temporales.
type Params = Vec<(String, String)>;

#[derive(Clone)]
pub struct ComicVineClient {
    api_key: Arc<String>,
    base_url: Arc<String>,
    client: reqwest::Client,
    last_request: Arc<Mutex<Instant>>,
}

impl ComicVineClient {
    pub fn new(api_key: impl Into<String>, base_url: Option<String>) -> Result<Self> {
        let api_key = api_key.into();
        if api_key.is_empty() {
            bail!("La API Key no puede estar vacía.");
        }

        let mut base = base_url.unwrap_or_else(|| BASE_URL.to_string());
        if !base.ends_with('/') {
            base.push('/');
        }

        let client = reqwest::Client::builder()
            .user_agent("BabelcomicsR/1.0 (Rust)")
            .build()?;

        Ok(Self {
            api_key: Arc::new(api_key),
            base_url: Arc::new(base),
            client,
            last_request: Arc::new(Mutex::new(
                Instant::now() - REQUEST_INTERVAL,
            )),
        })
    }

    async fn wait_for_rate_limit(&self) {
        let mut last = self.last_request.lock().await;
        let elapsed = last.elapsed();
        if elapsed < REQUEST_INTERVAL {
            tokio::time::sleep(REQUEST_INTERVAL - elapsed).await;
        }
        *last = Instant::now();
    }

    /// Construye la URL manualmente para evitar que reqwest codifique los `:` y `+`
    /// en los parámetros de filtro, que ComicVine necesita sin codificar.
    fn build_url(&self, endpoint: &str, params: &Params) -> String {
        // Asegurar que el endpoint termine en / si no lo tiene
        let endpoint_fmt = if endpoint.ends_with('/') || endpoint.is_empty() {
            endpoint.to_string()
        } else {
            format!("{}/", endpoint)
        };

        let mut url = format!(
            "{}{}?api_key={}&format=json",
            self.base_url, endpoint_fmt, self.api_key
        );
        for (k, v) in params {
            url.push_str(&format!("&{}={}", k, v));
        }
        url
    }

    async fn make_request(&self, endpoint: &str, params: Params) -> Option<Value> {
        self.wait_for_rate_limit().await;

        let url = self.build_url(endpoint, &params);
        tracing::debug!("ComicVine → {}", url);

        let response = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("ComicVine conexión fallida: {}", e);
                return None;
            }
        };

        if !response.status().is_success() {
            tracing::error!("ComicVine error HTTP: {}", response.status());
            return None;
        }

        response.json::<Value>().await.ok()
    }

    /// Valida la conexión realizando una petición mínima (límite 1) al endpoint de volúmenes.
    pub async fn validate(&self) -> Result<()> {
        let params = vec![("limit".to_string(), "1".to_string())];
        match self.make_request("volumes", params).await {
            Some(json) => {
                if let Some(error) = json.get("error").and_then(|e| e.as_str()) {
                    if error != "OK" {
                        bail!("ComicVine respondió con error: {}", error);
                    }
                }
                Ok(())
            }
            None => bail!("No se pudo conectar con ComicVine o la respuesta fue inválida."),
        }
    }

    // ── Publishers ────────────────────────────────────────────────────────────

    pub async fn get_publishers(
        &self,
        limit: usize,
        offset: usize,
        name_filter: Option<&str>,
    ) -> Vec<Value> {
        let mut params: Params = vec![
            ("limit".into(), limit.to_string()),
            ("offset".into(), offset.to_string()),
        ];
        if let Some(name) = name_filter {
            params.push(("filter".into(), format!("name:{}", name)));
        }

        self.make_request("publishers/", params)
            .await
            .and_then(|d| d["results"].as_array().cloned())
            .unwrap_or_default()
    }

    pub async fn get_publisher_details(&self, publisher_id: &str) -> Option<Value> {
        let data = self
            .make_request(&format!("publisher/{}/", publisher_id), vec![])
            .await?;
        Some(data["results"].clone())
    }

    // ── Volumes ───────────────────────────────────────────────────────────────

    pub async fn get_volumes(
        &self,
        query: Option<&str>,
        publisher_id: Option<&str>,
    ) -> Vec<Value> {
        let filter_str = build_filter(&[
            query.map(|q| format!("name:{}", q)),
            publisher_id.map(|p| format!("publisher:{}", p)),
        ]);

        // Primera página
        let first_params = page_params(API_RESULTS_LIMIT, 0, &filter_str);
        let first_data = match self.make_request("volumes/", first_params).await {
            Some(d) => d,
            None => return vec![],
        };

        let total = first_data["number_of_total_results"]
            .as_u64()
            .unwrap_or(0) as usize;

        let mut all: Vec<Value> = first_data["results"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        if total <= all.len() {
            return all;
        }

        // Páginas restantes en paralelo (limitadas por MAX_CONCURRENT)
        let num_pages = (total + API_RESULTS_LIMIT - 1) / API_RESULTS_LIMIT;
        let mut set = tokio::task::JoinSet::new();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT));

        for page in 1..num_pages {
            let offset = page * API_RESULTS_LIMIT;
            if offset >= total {
                break;
            }
            let client = self.clone();
            let filter = filter_str.clone();
            let sem = semaphore.clone();

            set.spawn(async move {
                let _permit = sem.acquire().await;
                let params = page_params(API_RESULTS_LIMIT, offset, &filter);
                client.make_request("volumes/", params).await
            });
        }

        while let Some(res) = set.join_next().await {
            if let Ok(Some(data)) = res {
                if let Some(results) = data["results"].as_array() {
                    all.extend(results.clone());
                }
            }
        }

        all
    }

    pub async fn get_volume_details(&self, volume_id: &str) -> Option<Value> {
        let id = format_resource_id(volume_id, "volume");
        let data = self
            .make_request(&format!("volume/{}/", id), vec![])
            .await?;
        Some(data["results"].clone())
    }

    // ── Issues ────────────────────────────────────────────────────────────────

    pub async fn get_issues(
        &self,
        limit: usize,
        offset: usize,
        query: Option<&str>,
        volume_id: Option<&str>,
        publisher_id: Option<&str>,
    ) -> Vec<Value> {
        let filter_str = build_filter(&[
            query.map(|q| format!("name:{}", q)),
            volume_id.map(|v| format!("volume:{}", v)),
            publisher_id.map(|p| format!("publisher:{}", p)),
        ]);

        let params = page_params(limit, offset, &filter_str);

        self.make_request("issues/", params)
            .await
            .and_then(|d| d["results"].as_array().cloned())
            .unwrap_or_default()
    }

    pub async fn get_issue_details(&self, issue_id: &str) -> Option<Value> {
        let id = format_resource_id(issue_id, "issue");
        let data = self
            .make_request(&format!("issue/{}/", id), vec![])
            .await?;
        Some(data["results"].clone())
    }

    /// Obtiene múltiples issues por ID en paralelo, en grupos de 100.
    pub async fn get_issues_by_ids(&self, ids: &[u64]) -> Vec<Value> {
        if ids.is_empty() {
            return vec![];
        }

        let mut set = tokio::task::JoinSet::new();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT));

        for chunk in ids.chunks(API_RESULTS_LIMIT) {
            let filter = format!(
                "id:{}",
                chunk
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join("|")
            );
            let client = self.clone();
            let sem = semaphore.clone();

            set.spawn(async move {
                let _permit = sem.acquire().await;
                let params: Params = vec![
                    ("limit".into(), API_RESULTS_LIMIT.to_string()),
                    ("offset".into(), "0".to_string()),
                    ("filter".into(), filter),
                ];
                client.make_request("issues/", params).await
            });
        }

        let mut all = vec![];
        while let Some(res) = set.join_next().await {
            if let Ok(Some(data)) = res {
                if let Some(results) = data["results"].as_array() {
                    all.extend(results.clone());
                }
            }
        }
        all
    }

    /// Obtiene todos los issues de un volumen, manejando la paginación secuencialmente.
    pub async fn get_volume_issues(&self, volume_id: &str) -> Vec<Value> {
        let mut all = vec![];
        let mut offset = 0usize;

        loop {
            let params: Params = vec![
                ("limit".into(), API_RESULTS_LIMIT.to_string()),
                ("offset".into(), offset.to_string()),
                ("filter".into(), format!("volume:{}", volume_id)),
            ];

            let data = match self.make_request("issues/", params).await {
                Some(d) => d,
                None => break,
            };

            let batch = match data["results"].as_array() {
                Some(b) if !b.is_empty() => b.clone(),
                _ => break,
            };

            let total = data["number_of_total_results"].as_u64().unwrap_or(0) as usize;
            all.extend(batch);

            if all.len() >= total {
                break;
            }

            offset += API_RESULTS_LIMIT;
        }

        all
    }

    // ── Search ────────────────────────────────────────────────────────────────

    pub async fn search(
        &self,
        query: &str,
        resource_type: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Vec<Value> {
        let mut params: Params = vec![
            ("query".into(), query.to_string()),
            ("limit".into(), limit.to_string()),
            ("offset".into(), offset.to_string()),
        ];
        if let Some(rt) = resource_type {
            params.push(("resources".into(), rt.to_string()));
        }

        self.make_request("search/", params)
            .await
            .and_then(|d| d["results"].as_array().cloned())
            .unwrap_or_default()
    }
}

// ── Utilidades ────────────────────────────────────────────────────────────────

/// Construye el string de filtros a partir de una lista de opcionales.
fn build_filter(parts: &[Option<String>]) -> String {
    parts
        .iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>()
        .join(",")
}

/// Construye los params comunes de paginación con filtro opcional.
fn page_params(limit: usize, offset: usize, filter: &str) -> Params {
    let mut params: Params = vec![
        ("limit".into(), limit.to_string()),
        ("offset".into(), offset.to_string()),
    ];
    if !filter.is_empty() {
        params.push(("filter".into(), filter.to_string()));
    }
    params
}
