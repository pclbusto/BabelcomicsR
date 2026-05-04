# Instalación

Esta guía cubre la instalación local de **BabelcomicsR** desde el código fuente.

## Requisitos

- Rust con `cargo`.
- GTK4 y Libadwaita.
- SQLite 3.
- Dependencias de compilación del sistema.

En Fedora:

```bash
sudo dnf install gtk4-devel libadwaita-devel sqlite-devel
```

En Ubuntu o Debian:

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev libsqlite3-dev
```

## Compilar

Desde la raíz del proyecto:

```bash
cargo build
```

Para ejecutar la aplicación:

```bash
cargo run
```

Para una compilación optimizada:

```bash
cargo run --release
```

## Documentación

La documentación está hecha con `mdbook`.

```bash
mdbook serve --open
```

Si `mdbook` no está instalado:

```bash
cargo install mdbook
```
