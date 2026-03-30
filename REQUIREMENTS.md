# Requisitos de BabelcomicsR

## Para compilar

| Herramienta | Versión mínima | Arch Linux |
|-------------|----------------|------------|
| Rust (stable) | 1.85+ (edition 2024) | `rustup` |
| pkg-config | cualquiera | `pkg-config` |
| GTK4 (headers) | 4.16+ | `gtk4` |
| libadwaita (headers) | 1.6+ | `libadwaita` |
| SQLite (headers) | 3.x | `sqlite` |

```bash
# Arch Linux
sudo pacman -S --needed gtk4 libadwaita sqlite pkg-config base-devel
```

## Para ejecutar (runtime)

| Herramienta | Por qué | Arch Linux |
|-------------|---------|------------|
| GTK4 | UI toolkit | `gtk4` |
| libadwaita | Componentes de UI | `libadwaita` |
| SQLite | Base de datos | `sqlite` |
| `unrar` | Leer archivos .cbr / .rar | `unrar` (AUR) |

```bash
# Arch Linux
sudo pacman -S gtk4 libadwaita sqlite
paru -S unrar   # o yay -S unrar
```

## Formatos de cómic soportados

| Extensión | Soporte | Dependencia |
|-----------|---------|-------------|
| `.cbz` / `.zip` | Completo | Ninguna (crate `zip`) |
| `.cbr` / `.rar` | Completo | `unrar` instalado en PATH |
| `.cb7` / `.7z` | Completo | Ninguna (crate `sevenz-rust`) |
| `.pdf` | Sin portada | — (pendiente de implementar) |

## Notas

- Sin `unrar`, los archivos `.cbr`/`.rar` se escanean e indexan correctamente,
  pero no se genera miniatura de portada para ellos.
- La base de datos se guarda en `~/.local/share/babelcomics/babelcomics.db`.
- Las miniaturas se guardan en `~/.local/share/babelcomics/thumbnails/`.
