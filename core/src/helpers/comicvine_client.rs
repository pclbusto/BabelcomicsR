use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use reqwest::header::{self, HeaderMap, HeaderValue};
use serde_json::Value;
use tokio::sync::Mutex;

/// Resultado tipado de una búsqueda de volúmenes en ComicVine.
#[derive(Debug, Clone)]
pub struct CvVolumeResult {
    pub cv_id: i64,
    pub name: String,
    pub publisher_name: String,
    pub image_url: String,
    pub start_year: String,
    pub count_of_issues: i64,
}

impl CvVolumeResult {
    fn from_value(v: &Value) -> Option<Self> {
        Some(CvVolumeResult {
            cv_id: v["id"].as_i64()?,
            name: v["name"].as_str().unwrap_or("").to_string(),
            publisher_name: v["publisher"]["name"].as_str().unwrap_or("").to_string(),
            image_url: v["image"]["medium_url"].as_str().unwrap_or("").to_string(),
            start_year: v["start_year"].as_str().unwrap_or("N/A").to_string(),
            count_of_issues: v["count_of_issues"].as_i64().unwrap_or(0),
        })
    }
}

/// Resultado tipado de una búsqueda de editoriales en ComicVine.
#[derive(Debug, Clone)]
pub struct CvPublisherResult {
    pub cv_id: i64,
    pub name: String,
    pub image_url: String,
    pub deck: String,
    pub location_city: String,
    pub count_of_issues: i64,
}

impl CvPublisherResult {
    fn from_value(v: &Value) -> Option<Self> {
        let count = v["count_of_issues"]
            .as_u64()
            .or_else(|| v["volume_credits"].as_array().map(|a| a.len() as u64))
            .unwrap_or(0) as i64;
        Some(CvPublisherResult {
            cv_id: v["id"].as_i64()?,
            name: v["name"].as_str().unwrap_or("").to_string(),
            image_url: v["image"]["medium_url"].as_str().unwrap_or("").to_string(),
            deck: v["deck"].as_str().unwrap_or("").to_string(),
            location_city: v["location_city"].as_str().unwrap_or("").to_string(),
            count_of_issues: count,
        })
    }
}

const BASE_URL: &str = "https://comicvine.gamespot.com/api/";
const API_RESULTS_LIMIT: usize = 100;
const MAX_CONCURRENT: usize = 5;
const REQUEST_INTERVAL: Duration = Duration::from_millis(500);

fn resource_prefix(resource_type: &str) -> Option<&'static str> {
    match resource_type {
        "volume" => Some("4050-"),
        "publisher" => Some("4010-"), // Prefijo real de publishers en Comic Vine
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

        // Cabeceras por defecto que imitan a un navegador moderno.
        // Comic Vine (Gamespot) puede devolver HTML de error o bloquear
        // clientes sin User-Agent creíble.
        let mut default_headers = HeaderMap::new();
        default_headers.insert(
            header::USER_AGENT,
            HeaderValue::from_static(
                "Mozilla/5.0 (X11; Linux x86_64; rv:128.0) Gecko/20100101 Firefox/128.0",
            ),
        );
        default_headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("application/json, text/javascript, */*; q=0.01"),
        );
        default_headers.insert(
            header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("es-ES,es;q=0.9,en-US;q=0.8,en;q=0.7"),
        );
        default_headers.insert(
            header::REFERER,
            HeaderValue::from_static("https://comicvine.gamespot.com/"),
        );
        // Indica al servidor que es una petición XHR, como hace el frontend web
        default_headers.insert(
            header::HeaderName::from_static("x-requested-with"),
            HeaderValue::from_static("XMLHttpRequest"),
        );

        let client = reqwest::Client::builder()
            .default_headers(default_headers)
            .cookie_store(true) // gestiona cookies de sesión automáticamente
            .timeout(Duration::from_secs(30))
            .tcp_keepalive(Duration::from_secs(60))
            .build()?;

        Ok(Self {
            api_key: Arc::new(api_key),
            base_url: Arc::new(base),
            client,
            last_request: Arc::new(Mutex::new(Instant::now() - REQUEST_INTERVAL)),
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

        let mut url = format!("{}{}?api_key={}", self.base_url, endpoint_fmt, self.api_key);
        for (k, v) in params {
            // Saneamiento simple: ComicVine necesita los espacios como %20
            // pero NO queremos que otros caracteres como : o , se codifiquen
            // ni que el % de %20 se convierta en %25.
            let encoded_v = v.replace(" ", "%20");
            url.push_str(&format!("&{}={}", k, encoded_v));
        }
        url.push_str("&format=json");
        url
    }

    async fn make_request(&self, endpoint: &str, params: Params) -> Option<Value> {
        self.wait_for_rate_limit().await;

        let url = self.build_url(endpoint, &params);
        // Siempre visible en info para poder depurar fácilmente
        tracing::info!("ComicVine → {}", url);

        let response = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!("ComicVine conexión fallida: {}", e);
                return None;
            }
        };

        let status = response.status();
        if !status.is_success() {
            // Leer el cuerpo para ver el mensaje de error de la API
            let body = response
                .text()
                .await
                .unwrap_or_else(|_| "<sin cuerpo>".into());
            tracing::error!("ComicVine error HTTP {} para {}: {}", status, url, body);
            return None;
        }

        let json = match response.json::<Value>().await {
            Ok(j) => j,
            Err(e) => {
                tracing::error!("ComicVine JSON inválido desde {}: {}", url, e);
                return None;
            }
        };

        // Comprobar el campo "error" de la respuesta de Comic Vine
        if let Some(api_err) = json.get("error").and_then(|e| e.as_str()) {
            if api_err != "OK" {
                tracing::error!("ComicVine API error '{}' para {}", api_err, url);
                return None;
            }
        }

        Some(json)
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
    ) -> Vec<CvPublisherResult> {
        let mut params: Params = vec![
            ("limit".into(), limit.to_string()),
            ("offset".into(), offset.to_string()),
        ];
        if let Some(name) = name_filter {
            params.push(("filter".into(), format!("name:{}", sanitize_query(name))));
        }

        self.make_request("publishers/", params)
            .await
            .and_then(|d| d["results"].as_array().cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(CvPublisherResult::from_value)
            .collect()
    }

    pub async fn get_publisher_details(&self, publisher_id: &str) -> Option<Value> {
        // El endpoint acepta el ID desnudo: /publisher/{id}/ (sin prefijo 4010-)
        let data = self
            .make_request(&format!("publisher/{}/", publisher_id), vec![])
            .await?;
        Some(data["results"].clone())
    }

    // ── Volumes ───────────────────────────────────────────────────────────────

    pub async fn get_volumes(&self, query: Option<&str>, publisher_id: Option<&str>) -> Vec<CvVolumeResult> {
        let filter_str = build_filter(&[
            query.map(|q| format!("name:{}", sanitize_query(q))),
            publisher_id.map(|p| format!("publisher:{}", p)),
        ]);

        // Primera página
        let first_params = page_params(API_RESULTS_LIMIT, 0, &filter_str);
        let first_data = match self.make_request("volumes/", first_params).await {
            Some(d) => d,
            None => return vec![],
        };

        let total = first_data["number_of_total_results"].as_u64().unwrap_or(0) as usize;

        let mut all: Vec<Value> = first_data["results"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        if total > all.len() {
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
        }

        all.iter().filter_map(CvVolumeResult::from_value).collect()
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
        let data = self.make_request(&format!("issue/{}/", id), vec![]).await?;
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
            ("query".into(), sanitize_query(query)),
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

/// Codifica los caracteres no seguros para URL en el término de búsqueda del usuario.
/// Codifica espacios como %20 pero deja intactos los separadores de filtro (: | ,).
fn sanitize_query(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            ' ' => vec!['%', '2', '0'],
            '%' => vec!['%', '2', '5'],
            '#' => vec!['%', '2', '3'],
            '&' => vec!['%', '2', '6'],
            '+' => vec!['%', '2', 'B'],
            '?' => vec!['%', '3', 'F'],
            c => vec![c],
        })
        .collect()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn print_comicvine_query_url() {
        // Para que la URL sea funcional en tu navegador, pon tu API Key aquí.
        let api_key = "TU_API_KEY_AQUI";
        let client = ComicVineClient::new(api_key, None).unwrap();

        // Simular búsqueda de "The Amazing Spider-Man" en Marvel (ID 31)
        let query = Some("The Amazing Spider-Man");
        let publisher_id = Some("31");

        let filter_str = build_filter(&[
            query.map(|q| format!("name:{}", q)),
            publisher_id.map(|p| format!("publisher:{}", p)),
        ]);

        let params = page_params(100, 0, &filter_str);
        let url = client.build_url("volumes/", &params);

        println!("\n\n🔗 --- COPIA ESTA URL AL NAVEGADOR (VOLÚMENES) ---");
        println!("{}", url);
        println!("--------------------------------------\n");
    }

    #[test]
    fn print_publisher_query_url() {
        let api_key = "TU_API_KEY_AQUI";
        let client = ComicVineClient::new(api_key, None).unwrap();

        // Simular búsqueda de la editorial "DC Comics"
        let name_filter = Some("DC Comics");
        let mut params: Params = vec![
            ("limit".into(), "50".to_string()),
            ("offset".into(), "0".to_string()),
        ];
        if let Some(name) = name_filter {
            params.push(("filter".into(), format!("name:{}", sanitize_query(name))));
        }

        let url = client.build_url("publishers/", &params);

        println!("\n\n🔗 --- COPIA ESTA URL AL NAVEGADOR (EDITORIALES) ---");
        println!("{}", url);
        println!("--------------------------------------\n");
    }
}
