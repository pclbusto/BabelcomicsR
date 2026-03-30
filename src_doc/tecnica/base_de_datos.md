# Base de Datos (SQLx / SQLite)

**BabelcomicsR** utiliza **SQLite** como motor de almacenamiento local, gestionado a través de **SQLx**. Esta elección combina la ligereza de una base de datos embebida con la robustez de las validaciones en tiempo de compilación que ofrece Rust.

## Estructura del Modelo de Datos

El esquema está diseñado para manejar la jerarquía clásica del mundo del cómic, permitiendo una separación clara entre la información editorial y los archivos físicos en disco.

### Entidades Principales

-   **Publishers (Editoriales):** Almacena información de la editorial, incluyendo su ID de ComicVine para sincronización.
-   **Volumes (Series):** Agrupa los números (issues) bajo una cabecera común (ej: *Action Comics*). Incluye metadatos como el año de inicio y la cantidad total de números.
-   **Comicbooks Info (Metadatos):** Representa la información "teórica" de un número (número, resumen, calificación).
-   **Comicbooks (Archivos Físicos):** Representa el archivo real en el sistema de archivos (`.cbz`, `.cbr`). Está vinculado a una entrada de *Comicbooks Info*.
-   **Setups:** Almacena la configuración global de la aplicación (API keys, preferencias de interfaz, directorios de escaneo).

## Integración con SQLx

La ventaja crítica de usar **SQLx** en este proyecto es el **tipado fuerte** de las consultas. Utilizamos las macros `query!` y `query_as!` para asegurar que cualquier error en el SQL o en los tipos de datos se detecte durante la compilación, no en tiempo de ejecución.

### Ejemplo de Consulta Tipada

```rust
pub async fn get_setup(&self) -> Result<Setup> {
    let row = sqlx::query_as!(
        Setup,
        "SELECT setupkey, thumbnail_size, modo_oscuro FROM setups WHERE setupkey = ?",
        "default"
    )
    .fetch_one(self.pool)
    .await?;
    Ok(row)
}
```

## Migraciones y Robustez del Esquema

Aunque **BabelcomicsR** se encuentra en sus primeras etapas de desarrollo, utiliza un sistema de **migraciones SQL versionadas** desde el inicio. Esto establece una base sólida para el crecimiento del software:

1.  **Inmutabilidad del Historial:** Cada cambio en la estructura de datos se registra cronológicamente en archivos `.sql` numerados.
2.  **Consistencia en el Desarrollo:** Asegura que todos los entornos (desarrollo, pruebas y producción futura) trabajen exactamente con la misma estructura de tablas.
3.  **Preparación para el Futuro:** Cuando se añadan nuevas funcionalidades, el sistema podrá actualizar las bases de datos de los usuarios de forma transparente, sin pérdida de datos.

## Modo Offline

Para facilitar el desarrollo y la compilación en sistemas CI/CD, utilizamos el modo **offline** de SQLx (`SQLX_OFFLINE=true`). Esto guarda una caché de los metadatos de las consultas en la carpeta `.sqlx/`, permitiendo compilar el proyecto sin necesidad de tener una base de datos activa.
