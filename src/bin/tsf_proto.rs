use std::collections::VecDeque;
use std::f32::consts::PI;
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
    std: Vec<f32>,
}

#[derive(Debug, Clone)]
struct ScoredItem {
    path: String,
    distance: f32,
}

#[derive(Debug, Clone, Copy)]
struct BinaryStats {
    components: usize,
    holes: usize,
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
            let sampled = self.sample_topology(&diffused);
            features.extend(self.curves_to_landscape_features(&sampled.0));
            features.extend(self.curves_to_landscape_features(&sampled.1));
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
                    let north = prev[((y.saturating_sub(1)) * w + x) as usize];
                    let south =
                        prev[((y.min(h - 1).saturating_add(1).min(h - 1)) * w + x) as usize];
                    let west = prev[(y * w + x.saturating_sub(1)) as usize];
                    let east = prev[(y * w + x.min(w - 1).saturating_add(1).min(w - 1)) as usize];

                    let dn = north - center;
                    let ds = south - center;
                    let dw = west - center;
                    let de = east - center;

                    let cn = 1.0 / (1.0 + (dn / k).powi(2));
                    let cs = 1.0 / (1.0 + (ds / k).powi(2));
                    let cw = 1.0 / (1.0 + (dw / k).powi(2));
                    let ce = 1.0 / (1.0 + (de / k).powi(2));

                    let updated = center + 0.18 * (cn * dn + cs * ds + cw * dw + ce * de);
                    current[idx] = updated.clamp(0.0, 1.0);
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
        let mut h0_curve = Vec::with_capacity(self.n_landscape_samples);
        let mut h1_curve = Vec::with_capacity(self.n_landscape_samples);

        let denom = (self.n_landscape_samples.saturating_sub(1)).max(1) as f32;
        for i in 0..self.n_landscape_samples {
            let t = i as f32 / denom;
            let threshold = (t * 255.0).round() as u8;
            let binary_low = threshold_image_below(image, threshold);
            let binary_high = threshold_image_above(image, threshold);
            let stats_low = binary_topology_stats(&binary_low);
            let stats_high = binary_topology_stats(&binary_high);
            h0_curve.push(stats_low.components as f32);
            h1_curve.push(stats_high.holes as f32);
        }

        normalize_curve(&mut h0_curve);
        normalize_curve(&mut h1_curve);
        (h0_curve, h1_curve)
    }

    fn curves_to_landscape_features(&self, curve: &[f32]) -> Vec<f32> {
        let n = curve.len();
        if n == 0 {
            return vec![0.0; self.n_landscape_functions * self.n_landscape_samples];
        }

        let smooth = smooth_curve(curve);
        let mut levels = vec![vec![0.0f32; n]; self.n_landscape_functions];

        for i in 0..n {
            let left = if i > 0 { smooth[i - 1] } else { smooth[i] };
            let right = if i + 1 < n { smooth[i + 1] } else { smooth[i] };
            let rise = (smooth[i] - left).max(0.0);
            let fall = (smooth[i] - right).max(0.0);
            let curvature = (2.0 * smooth[i] - left - right).abs();

            levels[0][i] = smooth[i];
            if self.n_landscape_functions > 1 {
                levels[1][i] = rise.max(fall);
            }
            if self.n_landscape_functions > 2 {
                levels[2][i] = curvature;
            }
            if self.n_landscape_functions > 3 {
                levels[3][i] = (rise + fall) * 0.5;
            }
            if self.n_landscape_functions > 4 {
                levels[4][i] = local_energy(&smooth, i);
            }
        }

        let mut features =
            Vec::with_capacity(self.n_landscape_functions * self.n_landscape_samples);
        for level in levels {
            let mut sampled = resample_curve(&level, self.n_landscape_samples);
            normalize_curve(&mut sampled);
            features.extend(sampled);
        }
        features
    }

    fn fit_embedding_dim(&self, mut features: Vec<f32>) -> Vec<f32> {
        if features.len() > self.embedding_dim {
            features.truncate(self.embedding_dim);
            return features;
        }
        if features.len() < self.embedding_dim {
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
        let (mean, std) = compute_zscore_params(&embeddings);
        let normalized = embeddings
            .into_iter()
            .map(|emb| zscore_normalize(&emb, &mean, &std))
            .collect::<Vec<_>>();

        for (item, norm) in items.iter_mut().zip(normalized) {
            item.embedding = norm;
        }

        Ok(StoredIndex {
            config: self.clone(),
            items,
            mean,
            std,
        })
    }

    fn search_similar(
        &self,
        index: &StoredIndex,
        query_path: &Path,
        k: usize,
    ) -> Result<Vec<ScoredItem>> {
        let query_embedding = self.compute_embedding(query_path)?;
        let query_norm = zscore_normalize(&query_embedding, &index.mean, &index.std);

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
            fs::write(&out, serde_json::to_vec_pretty(&index)?)
                .with_context(|| format!("No se pudo escribir {}", out.display()))?;
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
            let index: StoredIndex = serde_json::from_slice(
                &fs::read(index_path).with_context(|| format!("No se pudo leer {}", index_path))?,
            )?;
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
        "Uso:
  cargo run --bin tsf_proto -- demo
  cargo run --bin tsf_proto -- embed <imagen>
  cargo run --bin tsf_proto -- index <directorio> [--out archivo.json]
  cargo run --bin tsf_proto -- search <indice.json> <imagen> [--k N]"
    );
}

fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].clone())
}

fn collect_images(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = fs::read_dir(dir)
        .with_context(|| format!("No se pudo leer {}", dir.display()))?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .map(|ext| {
                        matches!(
                            ext.to_ascii_lowercase().as_str(),
                            "png" | "jpg" | "jpeg" | "webp" | "bmp"
                        )
                    })
                    .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn threshold_image_below(image: &GrayImage, threshold: u8) -> Vec<u8> {
    image
        .pixels()
        .map(|pixel| u8::from(pixel[0] <= threshold))
        .collect()
}

fn threshold_image_above(image: &GrayImage, threshold: u8) -> Vec<u8> {
    image
        .pixels()
        .map(|pixel| u8::from(pixel[0] >= threshold))
        .collect()
}

fn binary_topology_stats(binary: &[u8]) -> BinaryStats {
    let side = (binary.len() as f64).sqrt() as usize;
    if side == 0 || side * side != binary.len() {
        return BinaryStats {
            components: 0,
            holes: 0,
        };
    }

    let components = count_components(binary, side, side, 1);
    let background = count_components(binary, side, side, 0);
    let holes = background.saturating_sub(1);

    BinaryStats { components, holes }
}

fn count_components(binary: &[u8], width: usize, height: usize, target: u8) -> usize {
    let mut visited = vec![false; binary.len()];
    let mut count = 0;

    for y in 0..height {
        for x in 0..width {
            let idx = y * width + x;
            if visited[idx] || binary[idx] != target {
                continue;
            }
            count += 1;
            visited[idx] = true;
            let mut queue = VecDeque::from([(x, y)]);
            while let Some((cx, cy)) = queue.pop_front() {
                for (nx, ny) in neighbors4(cx, cy, width, height) {
                    let nidx = ny * width + nx;
                    if !visited[nidx] && binary[nidx] == target {
                        visited[nidx] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }
        }
    }

    count
}

fn neighbors4(x: usize, y: usize, width: usize, height: usize) -> [(usize, usize); 4] {
    [
        (x.saturating_sub(1), y),
        ((x + 1).min(width - 1), y),
        (x, y.saturating_sub(1)),
        (x, (y + 1).min(height - 1)),
    ]
}

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

fn smooth_curve(curve: &[f32]) -> Vec<f32> {
    let n = curve.len();
    let mut out = vec![0.0; n];
    for i in 0..n {
        let left = if i > 0 { curve[i - 1] } else { curve[i] };
        let center = curve[i];
        let right = if i + 1 < n { curve[i + 1] } else { curve[i] };
        out[i] = 0.25 * left + 0.5 * center + 0.25 * right;
    }
    out
}

fn local_energy(curve: &[f32], i: usize) -> f32 {
    let n = curve.len();
    let prev = if i > 0 { curve[i - 1] } else { curve[i] };
    let next = if i + 1 < n { curve[i + 1] } else { curve[i] };
    let grad = (next - prev).abs();
    let osc = ((i as f32 / n.max(1) as f32) * PI).sin().abs();
    grad * (1.0 + osc)
}

fn resample_curve(curve: &[f32], out_len: usize) -> Vec<f32> {
    if curve.is_empty() || out_len == 0 {
        return Vec::new();
    }
    if curve.len() == out_len {
        return curve.to_vec();
    }

    let mut out = Vec::with_capacity(out_len);
    let scale = (curve.len() - 1) as f32 / (out_len - 1).max(1) as f32;
    for i in 0..out_len {
        let pos = i as f32 * scale;
        let left = pos.floor() as usize;
        let right = pos.ceil() as usize;
        if left == right {
            out.push(curve[left]);
        } else {
            let t = pos - left as f32;
            out.push(curve[left] * (1.0 - t) + curve[right] * t);
        }
    }
    out
}

fn compute_zscore_params(embeddings: &[Vec<f32>]) -> (Vec<f32>, Vec<f32>) {
    let dim = embeddings.first().map(|v| v.len()).unwrap_or(0);
    let mut mean = vec![0.0; dim];
    let mut std = vec![0.0; dim];
    if embeddings.is_empty() {
        return (mean, vec![1.0; dim]);
    }

    for emb in embeddings {
        for (i, value) in emb.iter().enumerate() {
            mean[i] += *value;
        }
    }
    for value in &mut mean {
        *value /= embeddings.len() as f32;
    }
    for emb in embeddings {
        for (i, value) in emb.iter().enumerate() {
            std[i] += (*value - mean[i]).powi(2);
        }
    }
    for value in &mut std {
        *value = (*value / embeddings.len() as f32).sqrt().max(1e-6);
    }
    (mean, std)
}

fn zscore_normalize(embedding: &[f32], mean: &[f32], std: &[f32]) -> Vec<f32> {
    embedding
        .iter()
        .enumerate()
        .map(|(i, value)| (*value - mean[i]) / std[i])
        .collect()
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut na = 0.0;
    let mut nb = 0.0;
    for (va, vb) in a.iter().zip(b.iter()) {
        dot += va * vb;
        na += va * va;
        nb += vb * vb;
    }
    let denom = na.sqrt() * nb.sqrt();
    if denom <= 1e-12 {
        return 1.0;
    }
    1.0 - (dot / denom).clamp(-1.0, 1.0)
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

    println!("============================================================");
    println!("Topological Signature Flow - Demo Rust");
    println!("============================================================");

    let index = tsf.build_index(&paths)?;
    println!(
        "Embeddings computados: {} x {}",
        index.items.len(),
        tsf.embedding_dim
    );

    let query = temp_dir.join("ring.png");
    let results = tsf.search_similar(&index, &query, 3)?;

    println!("\nSimilares a ring:");
    for (i, item) in results.iter().enumerate() {
        let name = Path::new(&item.path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?");
        println!("  {}. {} (distancia: {:.4})", i + 1, name, item.distance);
    }

    println!("\nMatriz de distancias:");
    print!("            ");
    for item in &index.items {
        let name = Path::new(&item.path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?");
        print!("{:>10}", truncate_label(name));
    }
    println!();

    for row in &index.items {
        let row_name = Path::new(&row.path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?");
        print!("{:>12}", truncate_label(row_name));
        for col in &index.items {
            print!("{:>10.4}", cosine_distance(&row.embedding, &col.embedding));
        }
        println!();
    }

    println!("\nImágenes demo en {}", temp_dir.display());
    Ok(())
}

fn truncate_label(s: &str) -> String {
    s.chars().take(9).collect()
}

fn generate_gradient(width: u32, height: u32) -> GrayImage {
    ImageBuffer::from_fn(width, height, |x, _| {
        let v = ((x as f32 / (width - 1) as f32) * 255.0) as u8;
        Luma([v])
    })
}

fn generate_checkerboard(width: u32, height: u32, tile: u32) -> GrayImage {
    ImageBuffer::from_fn(width, height, |x, y| {
        let even = ((x / tile) + (y / tile)).is_multiple_of(2);
        Luma([if even { 32 } else { 224 }])
    })
}

fn generate_circle(width: u32, height: u32, radius: f32) -> GrayImage {
    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;
    ImageBuffer::from_fn(width, height, |x, y| {
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        let inside = dx * dx + dy * dy <= radius * radius;
        Luma([if inside { 220 } else { 24 }])
    })
}

fn generate_ring(width: u32, height: u32, inner: f32, outer: f32) -> GrayImage {
    let cx = width as f32 / 2.0;
    let cy = height as f32 / 2.0;
    ImageBuffer::from_fn(width, height, |x, y| {
        let dx = x as f32 - cx;
        let dy = y as f32 - cy;
        let d2 = dx * dx + dy * dy;
        let inside = d2 >= inner * inner && d2 <= outer * outer;
        Luma([if inside { 230 } else { 18 }])
    })
}

fn generate_stripes(width: u32, height: u32, period: u32) -> GrayImage {
    ImageBuffer::from_fn(width, height, |x, _| {
        let band = (x / period).is_multiple_of(2);
        Luma([if band { 210 } else { 40 }])
    })
}

fn generate_noise(width: u32, height: u32) -> GrayImage {
    let mut state = 0x1234_5678u64;
    ImageBuffer::from_fn(width, height, |_x, _y| {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        Luma([((state >> 24) & 0xff) as u8])
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffusion_preserves_dimensions() {
        let img = generate_gradient(32, 24);
        let tsf = TopologicalSignatureFlow::default();
        let out = tsf.anisotropic_diffusion(&img, 3, 0.8);
        assert_eq!(img.dimensions(), out.dimensions());
    }

    #[test]
    fn count_components_detects_two_islands() {
        let width = 4usize;
        let height = 4usize;
        let binary = vec![
            1, 1, 0, 0, //
            1, 1, 0, 0, //
            0, 0, 1, 1, //
            0, 0, 1, 1,
        ];
        assert_eq!(count_components(&binary, width, height, 1), 2);
    }

    #[test]
    fn holes_detect_ring_shape() {
        let img = generate_ring(64, 64, 12.0, 20.0);
        let binary = threshold_image_above(&img, 120);
        let stats = binary_topology_stats(&binary);
        assert!(stats.holes >= 1);
    }

    #[test]
    fn embedding_has_requested_dimension() {
        let tsf = TopologicalSignatureFlow {
            embedding_dim: 96,
            ..Default::default()
        };
        let fitted = tsf.fit_embedding_dim(vec![1.0; 32]);
        assert_eq!(fitted.len(), 96);
    }
}
