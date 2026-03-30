# Arquitectura Técnica

La elección de tecnologías en **BabelcomicsR** no es casual; responde directamente a los objetivos de rendimiento y estabilidad del proyecto.

## Componentes del Sistema

| Componente | Tecnología | Razón de Elección |
| :--- | :--- | :--- |
| **Núcleo (Backend)** | Rust | Rendimiento cercano a C++ sin los riesgos de memoria. Ideal para procesar grandes librerías. |
| **Base de Datos** | SQLite (SQLx) | Almacenamiento local ligero, robusto y portable. |
| **Interfaz (Frontend)** | GTK4 / Libadwaita | Componentes modernos, adaptativos y con aceleración por hardware. |
| **Procesamiento de Archivos** | `zip-rs` / `tar-rs` | Manejo nativo de contenedores `.cbz` y `.cbr` con baja sobrecarga. |

---

## Ventajas de la Migración a Rust

> **Nota de Desarrollo:** Al migrar desde Python, hemos observado una reducción drástica en los tiempos de escaneo inicial de bibliotecas masivas (más de 1,000 ejemplares), gracias al manejo de hilos nativos y la ausencia de un Garbage Collector pesado.

La transición de implementaciones anteriores en Python o .NET ha permitido:
- **Paralelismo Real:** Aprovechamiento de todos los núcleos del procesador durante el escaneo y la generación de thumbnails.
- **Binario Estático:** Una distribución mucho más sencilla en Linux, sin dependencias externas de lenguajes interpretados.
- **Gestión Precisa de Memoria:** Control total sobre cuándo se liberan los recursos, crucial al manejar imágenes de alta resolución en la interfaz.
