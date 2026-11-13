use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use image::imageops::FilterType;
use image::{GrayImage, ImageBuffer, Luma};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TopologicalSignatureFlow {
    diffusion_steps: usize,
    n_landscape_functions: usize,
    n_landscape_samples: usize,
    embedding_dim: usize,
    resize_to: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct IndexedItem {
    path: String,
    embedding: Vec<f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredIndex {
    config: TopologicalSignatureFlow,
    items: Vec<IndexedItem>,
    mean: Vec<f32>,
    stdev: Vec<f32>,
}

#[derive(Debug, Clone)]
struct ScoredItem {
    path: String,
    distance: f32,
}

impl Default for TopologicalSignatureFlow {
    fn default() -> Self {
        Self {
            diffusion_steps: 5,
            n_landscape_functions: 3,
            n_landscape_samples: 50,
            embedding_dim: 128,
            resize_to: 128,
        }
    }
}

impl TopologicalSignatureFlow {
    fn compute_embedding(&self, image_path: &Path) -> Result<Vec<f32>> {
        let img = image::open(image_path)
            .with_context(|| format!("No se pudo cargar {}", image_path.display()))?;
        let gray = img.to_luma8();
        let resized =
            image::imageops::resize(&gray, self.resize_to, self.resize_to, FilterType::Triangle);

        let mut features = Vec::new();

        for step in 0..self.diffusion_steps {
            let iterations = step + 1;
            let k = 0.5 + step as f32 * 0.15;
            let diffused = self.anisotropic_diffusion(&resized, iterations, k);
            let (h0, h1) = self.sample_topology(&diffused);
            features.extend(h0);
            features.extend(h1);
        }

        Ok(self.fit_embedding_dim(features))
    }

    fn anisotropic_diffusion(&self, image: &GrayImage, iterations: usize, k: f32) -> GrayImage {
        let (w, h) = image.dimensions();
        let mut current = vec![0.0f32; (w * h) as usize];
        for y in 0..h {
            for x in 0..w {
                current[(y * w + x) as usize] = image.get_pixel(x, y)[0] as f32 / 255.0;
            }
        }

        for _ in 0..iterations {
            let prev = current.clone();
            for y in 0..h {
                for x in 0..w {
                    let idx = (y * w + x) as usize;
                    let center = prev[idx];
                    let neighbors =
                        neighbors4_fixed(x as usize, y as usize, w as usize, h as usize);

                    let mut delta = 0.0;
                    for nidx in neighbors {
                        let val = prev[nidx];
                        let diff = val - center;
                        let coeff = 1.0 / (1.0 + (diff / k).powi(2));
                        delta += coeff * diff;
                    }
                    current[idx] = (center + 0.18 * delta).clamp(0.0, 1.0);
                }
            }
        }

        let mut out: GrayImage = ImageBuffer::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let val = (current[(y * w + x) as usize] * 255.0).round() as u8;
                out.put_pixel(x, y, Luma([val]));
            }
        }
        out
    }

    fn sample_topology(&self, image: &GrayImage) -> (Vec<f32>, Vec<f32>) {
        let (width, height) = image.dimensions();
        let (w, h) = (width as usize, height as usize);
        let pixels: Vec<f32> = image.pixels().map(|p| p[0] as f32 / 255.0).collect();

        // H0: filtración sublevel (componentes conectados)
        let pairs_h0 = compute_persistence_h0(&pixels, w, h);
        // H1: filtración superlevel (agujeros)
        let pixels_inv: Vec<f32> = pixels.iter().map(|&p| 1.0 - p).collect();
        let pairs_h1 = compute_persistence_h0(&pixels_inv, w, h);

        let h0_features = self.pairs_to_landscape(&pairs_h0);
        let h1_features = self.pairs_to_landscape(&pairs_h1);

        (h0_features, h1_features)
    }

    fn pairs_to_landscape(&self, pairs: &[(f32, f32)]) -> Vec<f32> {
        let n = self.n_landscape_samples;
        let mut sorted_pairs = pairs.to_vec();
        sorted_pairs.sort_by(|a, b| (b.1 - b.0).total_cmp(&(a.1 - a.0)));

        let mut features = Vec::with_capacity(self.n_landscape_functions * n);
        let denom = (n.saturating_sub(1)).max(1) as f32;

        for k in 0..self.n_landscape_functions {
            let mut landscape = vec![0.0f32; n];
            if k < sorted_pairs.len() {
                let (b, d) = sorted_pairs[k];
                for i in 0..n {
                    let t = i as f32 / denom;
                    landscape[i] = (0.0f32).max((t - b).min(d - t));
                }
                normalize_curve(&mut landscape);
            }
            features.extend(landscape);
        }
        features
    }

    fn fit_embedding_dim(&self, mut features: Vec<f32>) -> Vec<f32> {
        if features.len() > self.embedding_dim {
            features.truncate(self.embedding_dim);
        } else if features.len() < self.embedding_dim {
            features.resize(self.embedding_dim, 0.0);
        }
        features
    }

    fn build_index(&self, image_paths: &[PathBuf]) -> Result<StoredIndex> {
        let mut items = Vec::with_capacity(image_paths.len());
        println!(
            "Computando embeddings para {} imágenes...",
            image_paths.len()
        );

        for (i, path) in image_paths.iter().enumerate() {
            let embedding = self.compute_embedding(path)?;
            items.push(IndexedItem {
                path: path.to_string_lossy().into_owned(),
                embedding,
            });
            if (i + 1) % 10 == 0 || i + 1 == image_paths.len() {
                println!("  Procesadas {}/{}", i + 1, image_paths.len());
            }
        }

        let embeddings: Vec<Vec<f32>> = items.iter().map(|item| item.embedding.clone()).collect();
        let (mean, stdev) = compute_zscore_params(&embeddings);
        for item in &mut items {
            item.embedding = zscore_normalize(&item.embedding, &mean, &stdev);
        }

        Ok(StoredIndex {
            config: self.clone(),
            items,
            mean,
            stdev,
        })
    }

    fn search_similar(
        &self,
        index: &StoredIndex,
        query_path: &Path,
        k: usize,
    ) -> Result<Vec<ScoredItem>> {
        let query_embedding = self.compute_embedding(query_path)?;
        let query_norm = zscore_normalize(&query_embedding, &index.mean, &index.stdev);

        let mut scored = index
            .items
            .iter()
            .map(|item| ScoredItem {
                path: item.path.clone(),
                distance: cosine_distance(&query_norm, &item.embedding),
            })
            .collect::<Vec<_>>();

        scored.sort_by(|a, b| a.distance.total_cmp(&b.distance));
        scored.truncate(k.min(scored.len()));
        Ok(scored)
    }
}

// ---------------------------------------------------------------------------
// Algoritmo de Persistencia Topológica (H0) vía Union-Find
// ---------------------------------------------------------------------------

struct UnionFind {
    parent: Vec<usize>,
    birth: Vec<f32>,
}

impl UnionFind {
    fn new(n: usize, values: &[f32]) -> Self {
        Self {
            parent: (0..n).collect(),
            birth: values.to_vec(),
        }
    }

    fn find(&mut self, i: usize) -> usize {
        let mut root = i;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        // Path compression (iterativa para evitar stack overflow)
        let mut curr = i;
        while self.parent[curr] != root {
            let next = self.parent[curr];
            self.parent[curr] = root;
            curr = next;
        }
        root
    }

    fn union(&mut self, i: usize, j: usize, current_val: f32) -> Option<(f32, f32)> {
        let root_i = self.find(i);
        let root_j = self.find(j);
        if root_i != root_j {
            let (older, younger) = if self.birth[root_i] <= self.birth[root_j] {
                (root_i, root_j)
            } else {
                (root_j, root_i)
            };
            let birth = self.birth[younger];
            let death = current_val;
            self.parent[younger] = older;
            if death > birth {
                return Some((birth, death));
            }
        }
        None
    }
}

fn compute_persistence_h0(pixels: &[f32], w: usize, h: usize) -> Vec<(f32, f32)> {
    let mut indices: Vec<usize> = (0..pixels.len()).collect();
    indices.sort_by(|&a, &b| pixels[a].total_cmp(&pixels[b]));

    let mut uf = UnionFind::new(pixels.len(), pixels);
    let mut active = vec![false; pixels.len()];
    let mut pairs = Vec::new();

    for &idx in &indices {
        active[idx] = true;
        let val = pixels[idx];
        let x = idx % w;
        let y = idx / w;

        for nidx in neighbors4_fixed(x, y, w, h) {
            if active[nidx] {
                if let Some(pair) = uf.union(idx, nidx, val) {
                    pairs.push(pair);
                }
            }
        }
    }

    // Los componentes que no han muerto (raíces de UF) mueren al final de la filtración (1.0)
    for i in 0..pixels.len() {
        if uf.parent[i] == i && active[i] {
            pairs.push((uf.birth[i], 1.0));
        }
    }

    pairs
}

fn neighbors4_fixed(x: usize, y: usize, w: usize, h: usize) -> Vec<usize> {
    let mut v = Vec::with_capacity(4);
    if x > 0 {
        v.push(y * w + (x - 1));
    }
    if x + 1 < w {
        v.push(y * w + (x + 1));
    }
    if y > 0 {
        v.push((y - 1) * w + x);
    }
    if y + 1 < h {
        v.push((y + 1) * w + x);
    }
    v
}

// ---------------------------------------------------------------------------
// Helpers matemáticos y utilidades
// ---------------------------------------------------------------------------

fn normalize_curve(curve: &mut [f32]) {
    if curve.is_empty() {
        return;
    }
    let max = curve.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let min = curve.iter().copied().fold(f32::INFINITY, f32::min);
    let span = (max - min).max(1e-6);
    for value in curve.iter_mut() {
        *value = (*value - min) / span;
    }
}

fn compute_zscore_params(embeddings: &[Vec<f32>]) -> (Vec<f32>, Vec<f32>) {
    let dim = embeddings.first().map(|v| v.len()).unwrap_or(0);
    let mut mean = vec![0.0; dim];
    let mut stdev = vec![0.0; dim];
    if embeddings.is_empty() {
        return (mean, vec![1.0; dim]);
    }

    for emb in embeddings {
        for (i, &v) in emb.iter().enumerate() {
            mean[i] += v;
        }
    }
    for m in &mut mean {
        *m /= embeddings.len() as f32;
    }

    for emb in embeddings {
        for (i, &v) in emb.iter().enumerate() {
            stdev[i] += (v - mean[i]).powi(2);
        }
    }
    for s in &mut stdev {
        *s = (*s / embeddings.len() as f32).sqrt().max(1e-6);
    }

    (mean, stdev)
}

fn zscore_normalize(embedding: &[f32], mean: &[f32], stdev: &[f32]) -> Vec<f32> {
    embedding
        .iter()
        .enumerate()
        .map(|(i, &v)| (v - mean[i]) / stdev[i])
        .collect()
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let (mut dot, mut na, mut nb) = (0.0, 0.0, 0.0);
    for (&va, &vb) in a.iter().zip(b.iter()) {
        dot += va * vb;
        na += va * va;
        nb += vb * vb;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= 1e-12 {
        1.0
    } else {
        1.0 - (dot / denom).clamp(-1.0, 1.0)
    }
}

fn main() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.is_empty() {
        print_usage();
        return Ok(());
    }

    let tsf = TopologicalSignatureFlow::default();
    match args[0].as_str() {
        "embed" => {
            let path = args.get(1).context("Uso: tsf_proto embed <imagen>")?;
            let embedding = tsf.compute_embedding(Path::new(path))?;
            println!("{}", serde_json::to_string_pretty(&embedding)?);
        }
        "index" => {
            let dir = args
                .get(1)
                .context("Uso: tsf_proto index <dir> [--out archivo.json]")?;
            let out = parse_flag_value(&args[2..], "--out")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/tmp/tsf_index.json"));
            let images = collect_images(Path::new(dir))?;
            if images.is_empty() {
                bail!("No se encontraron imágenes en {}", dir);
            }
            let index = tsf.build_index(&images)?;
            fs::write(&out, serde_json::to_vec_pretty(&index)?)?;
            println!("Índice guardado en {}", out.display());
        }
        "search" => {
            let index_path = args
                .get(1)
                .context("Uso: tsf_proto search <indice.json> <imagen> [--k N]")?;
            let query_path = args
                .get(2)
                .context("Uso: tsf_proto search <indice.json> <imagen> [--k N]")?;
            let k = parse_flag_value(&args[3..], "--k")
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(5);
            let index: StoredIndex = serde_json::from_slice(&fs::read(index_path)?)?;
            let results = tsf.search_similar(&index, Path::new(query_path), k)?;
            for (i, item) in results.iter().enumerate() {
                println!("{:>2}. {:.4}  {}", i + 1, item.distance, item.path);
            }
        }
        "demo" => run_demo(&tsf)?,
        _ => print_usage(),
    }
    Ok(())
}

fn print_usage() {
    eprintln!(
        "Uso:\n  cargo run --bin tsf_proto -- demo\n  cargo run --bin tsf_proto -- embed <imagen>\n  cargo run --bin tsf_proto -- index <directorio> [--out archivo.json]\n  cargo run --bin tsf_proto -- search <indice.json> <imagen> [--k N]"
    );
}

fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == flag).map(|w| w[1].clone())
}

fn collect_images(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| {
                        matches!(
                            e.to_ascii_lowercase().as_str(),
                            "png" | "jpg" | "jpeg" | "webp" | "bmp"
                        )
                    })
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn run_demo(tsf: &TopologicalSignatureFlow) -> Result<()> {
    let temp_dir = std::env::temp_dir().join("tsf_demo_rs");
    fs::create_dir_all(&temp_dir)?;

    let samples = vec![
        ("gradient", generate_gradient(256, 256)),
        ("checkerboard", generate_checkerboard(256, 256, 16)),
        ("circle", generate_circle(256, 256, 72.0)),
        ("ring", generate_ring(256, 256, 40.0, 78.0)),
        ("stripes", generate_stripes(256, 256, 12)),
        ("noise", generate_noise(256, 256)),
    ];

    let mut paths = Vec::new();
    for (name, image) in &samples {
        let path = temp_dir.join(format!("{name}.png"));
        image.save(&path)?;
        paths.push(path);
    }

    let index = tsf.build_index(&paths)?;
    let query = temp_dir.join("ring.png");
    let results = tsf.search_similar(&index, &query, 3)?;

    println!("\nSimilares a ring:");
    for (i, item) in results.iter().enumerate() {
        println!(
            "  {}. {} (distancia: {:.4})",
            i + 1,
            Path::new(&item.path).file_stem().unwrap().to_str().unwrap(),
            item.distance
        );
    }
    Ok(())
}

fn generate_gradient(w: u32, h: u32) -> GrayImage {
    ImageBuffer::from_fn(w, h, |x, _| {
        Luma([((x as f32 / (w - 1) as f32) * 255.0) as u8])
    })
}
fn generate_checkerboard(w: u32, h: u32, t: u32) -> GrayImage {
    ImageBuffer::from_fn(w, h, |x, y| {
        Luma([if ((x / t) + (y / t)) % 2 == 0 {
            32
        } else {
            224
        }])
    })
}
fn generate_circle(w: u32, h: u32, r: f32) -> GrayImage {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    ImageBuffer::from_fn(w, h, |x, y| {
        let (dx, dy) = (x as f32 - cx, y as f32 - cy);
        Luma([if dx * dx + dy * dy <= r * r { 220 } else { 24 }])
    })
}
fn generate_ring(w: u32, h: u32, r1: f32, r2: f32) -> GrayImage {
    let (cx, cy) = (w as f32 / 2.0, h as f32 / 2.0);
    ImageBuffer::from_fn(w, h, |x, y| {
        let (dx, dy) = (x as f32 - cx, y as f32 - cy);
        let d2 = dx * dx + dy * dy;
        Luma([if d2 >= r1 * r1 && d2 <= r2 * r2 {
            230
        } else {
            18
        }])
    })
}
fn generate_stripes(w: u32, h: u32, p: u32) -> GrayImage {
    ImageBuffer::from_fn(w, h, |x, _| Luma([if (x / p) % 2 == 0 { 210 } else { 40 }]))
}
fn generate_noise(w: u32, h: u32) -> GrayImage {
    let mut s = 0x1234_5678u64;
    ImageBuffer::from_fn(w, h, |_, _| {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
        Luma([((s >> 24) & 0xff) as u8])
    })
}
