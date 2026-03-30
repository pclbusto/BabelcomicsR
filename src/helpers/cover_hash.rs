/// Perceptual hashing (difference hash / dHash) para comparación de similitud de portadas.
///
/// El hash resultante es un entero de 64 bits codificado como hex de 16 caracteres.
/// La distancia Hamming entre dos hashes indica su similitud:
///  0–5  → casi idénticas
///  6–15 → muy similares
/// 16–25 → similares
/// 26+   → diferentes

/// Calcula el dHash de una imagen a partir de sus bytes.
/// Redimensiona a 9×8 en escala de grises y compara píxeles adyacentes.
pub fn compute_hash(bytes: &[u8]) -> Option<String> {
    let img = image::load_from_memory(bytes).ok()?;
    let gray = img.into_luma8();
    let resized = image::imageops::resize(&gray, 9, 8, image::imageops::FilterType::Triangle);

    let mut hash: u64 = 0;
    for y in 0u32..8 {
        for x in 0u32..8 {
            let left = resized.get_pixel(x, y)[0];
            let right = resized.get_pixel(x + 1, y)[0];
            if left > right {
                hash |= 1u64 << (y * 8 + x);
            }
        }
    }

    Some(format!("{:016x}", hash))
}

/// Distancia Hamming entre dos hashes hex. None si alguno es inválido.
/// Rango: 0 (idénticas) – 64 (completamente distintas).
pub fn distance(a: &str, b: &str) -> Option<u32> {
    let a_bits = u64::from_str_radix(a, 16).ok()?;
    let b_bits = u64::from_str_radix(b, 16).ok()?;
    Some((a_bits ^ b_bits).count_ones())
}
