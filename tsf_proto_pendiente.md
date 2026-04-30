# Mejoras pendientes: tsf_proto.rs

Hallazgos del review contra el prototipo Python de TopologicalSignatureFlow.

## 1. Performance — union-find en lugar de BFS × N_samples

**Problema:** `count_components` se llama ~500 veces (una por threshold), haciendo BFS completo cada vez → O(N × n_samples).

**Solución:** reemplazar `sample_topology` + `count_components` + `binary_topology_stats` por una función `compute_persistence_h0(pixels, width, height)` que:
- Ordene los píxeles por intensidad ascendente (sublevel filtration)
- Use union-find para procesar aristas en orden → pares (birth, death) reales en un único paso O(N log N)
- Haga lo mismo en orden descendente para H1

Resultado: pares (b, d) de persistencia reales que alimentan correctamente los pasos siguientes.

## 2. Corrección — neighbors4 en bordes

**Problema:** en píxeles de borde, `neighbors4` devuelve el propio píxel como vecino (clamp al rango en lugar de filtrar). El BFS lo maneja via `visited` pero es semánticamente incorrecto.

**Solución:** filtrar los vecinos fuera de rango en lugar de saturar coordenadas.

```rust
fn neighbors4(idx: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = (idx % w, idx / w);
    let mut v = arrayvec::ArrayVec::<usize, 4>::new();
    if x > 0     { v.push(idx - 1); }
    if x + 1 < w { v.push(idx + 1); }
    if y > 0     { v.push(idx - w); }
    if y + 1 < h { v.push(idx + w); }
    v.into_iter()
}
```

## 3. Robustez — binary_topology_stats con dimensiones explícitas

**Problema:** la función asume imagen cuadrada (`let w = (n as f32).sqrt() as usize`), frágil si alguna vez se cambia el resize.

**Solución:** añadir parámetros `width: usize, height: usize` explícitos y eliminar la deducción interna.

## 4. Claridad — eliminar local_energy sinusoidal

**Problema:** `local_energy` aplica `sin(i/n × π)` como modulación de energía sin justificación geométrica conocida.

**Opciones:**
- Eliminarlo y usar la varianza local directa
- Documentar explícitamente el razonamiento si existe una razón válida

## 5. Consistencia — dirección de filtración H0 vs H1

**Problema:** H0 usa filtración sublevel (píxeles oscuros primero), H1 usa superlevel (píxeles claros primero). La asimetría no está documentada.

**Solución:** o ambas sublevel, o documentar la razón de la asimetría (e.g., "agujeros topológicos emergen en regiones claras").

## 6. Mejora matemática — Bubenik landscape real

**Problema:** `curves_to_landscape_features` usa heurísticas de derivadas en lugar de la fórmula de Bubenik.

**Una vez disponibles los pares (b, d) reales (punto 1):**

```
λ_k(t) = max(0, min(t - b, d - t))   para cada par (b, d), ordenados por persistencia
```

Esto da un vector de features matemáticamente fundamentado y comparable con literatura.

---

## Dependencias entre puntos

```
1 (union-find) → habilita → 6 (Bubenik landscape)
2 (neighbors4) → prerequisito para → 1
3 (dimensiones) → prerequisito para → 1
4 y 5 → independientes
```

Orden recomendado: 2 → 3 → 1 → 6, luego 4 y 5 en paralelo.
