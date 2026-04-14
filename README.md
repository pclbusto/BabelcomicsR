# 📚 Babelcomics

**Babelcomics** es un organizador y catalogador de cómics personales moderno, diseñado para ofrecer una experiencia premium y nativa en Linux. Desarrollado en **Rust**, utiliza inteligencia artificial y las últimas tecnologías de interfaz de usuario para que gestionar tu colección sea un placer.

![Captura de pantalla de la aplicación](https://raw.githubusercontent.com/Babelcomics/BabelcomicsR/main/assets/preview.png) *(Nota: reemplaza por una ruta real si existe)*

---

## ✨ Características Principales

- **🎨 Interfaz Moderna**: Construida con **GTK4** y **Libadwaita**, siguiendo las guías de diseño de GNOME para una integración perfecta en el escritorio Linux.
- **🤖 Catalogación Inteligente**: Utiliza modelos de IA (**CLIP visual embeddings** de OpenAI vía `candle`) para analizar las portadas de tus cómics y sugerir coincidencias exactas de metadatos.
- **🔍 Integración con ComicVine**: Sincronización completa con la API de ComicVine para obtener detalles de editoriales, series, volúmenes y artistas.
- **⚡ Alto Rendimiento**: Escaneo multi-hilo de librerías mediante **Rayon**, con soporte nativo para archivos comprimidos (.cbz, .cb7).
- **📊 Estadísticas Detalladas**: Visualiza el estado de tu colección, series completadas y posibles errores de catalogación desde un panel dedicado.
- **🔐 Privacidad y Seguridad**: Base de datos local **SQLite** y cifrado AES-GCM para tus llaves de API.

---

## 🛠️ Requisitos del Sistema

Para compilar y ejecutar Babelcomics, necesitarás:

- **Rust** (edición 2024 o superior)
- **GTK4** y **Libadwaita** desarrollos instalados en tu sistema:
  - En Fedora: `sudo dnf install gtk4-devel libadwaita-devel`
  - En Ubuntu/Debian: `sudo apt install libgtk-4-dev libadwaita-1-dev`
- **SQLite 3**

---

## 🚀 Instalación y Uso

1. **Clona el repositorio**:
   ```bash
   git clone https://github.com/tu-usuario/BabelcomicsR.git
   cd BabelcomicsR
   ```

2. **Compila y ejecuta**:
   ```bash
   cargo run --release
   ```

3. **Configuración inicial**:
   - Al iniciar, ve a la sección de configuración.
   - Introduce tu **API Key de ComicVine**.
   - Selecciona la carpeta donde guardas tus cómics.
   - ¡Inicia el escaneo y empieza a catalogar!

---

## 🏗️ Arquitectura Técnica

- **Lenguaje**: [Rust](https://www.rust-lang.org/)
- **Frontend**: [GTK4](https://www.gtk.org/) & [Libadwaita](https://gnome.pages.gitlab.gnome.org/libadwaita/)
- **Base de Datos**: [SQLite](https://www.sqlite.org/) con [SQLx](https://github.com/launchbadge/sqlx)
- **Runtime Async**: [Tokio](https://tokio.rs/)
- **Inteligencia Artificial**: [Candle](https://github.com/huggingface/candle) (ML framework de HuggingFace)
- **Procesamiento de Imágenes**: [image-rs](https://github.com/image-rs/image)

---

## 🤝 Contribuir

¡Las contribuciones son bienvenidas! Si tienes ideas para nuevas funciones o has encontrado un error, no dudes en abrir un *Issue* o enviar un *Pull Request*.

---

## 📄 Licencia

Este proyecto está bajo la Licencia MIT. Consulta el archivo `LICENSE` para más detalles.

---

<p align="center">Desarrollado con ❤️ para los amantes del noveno arte.</p>
