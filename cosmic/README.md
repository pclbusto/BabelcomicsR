# Babelcomics COSMIC

Frontend experimental de Babelcomics construido con
[libcosmic](https://github.com/pop-os/libcosmic). Comparte la base SQLite, los
repositorios y los thumbnails con la aplicación GTK de `../gui`; no crea una
biblioteca paralela.

## Estado

- Navegación COSMIC nativa: Cómics, Series, Editoriales y Actividad.
- Búsqueda contra los repositorios reales.
- Rejillas responsivas con portadas locales.
- Panel contextual con detalles y resumen de biblioteca.
- Configuración persistente para agregar o quitar una o varias carpetas a escanear.
- Escaneo de las carpetas configuradas, con seguimiento en la vista Actividad.
- Configuración ordenada por secciones: General, API ComicVine, carpetas a escanear,
  normalización del catálogo y opciones avanzadas.
- Filtros persistentes de catalogación para cómics y series, más un filtro de
  series con o sin actualizaciones obtenidas desde ComicVine.
- Eliminación con confirmación desde el detalle de cómics, volúmenes y editoriales.
- Multiselección de cómics con `Ctrl+clic`; `Ctrl+Espacio` abre el resumen agregado.
- Selección simple o múltiple de números dentro de una serie; `Ctrl+Espacio`
  abre su información y los cómics físicos relacionados, que pueden
  desclasificarse individualmente sin borrar el archivo.
- Apertura del lector `cosmic-reader` desde el detalle de un cómic físico y
  desde los archivos físicos relacionados con un número.
- Catalogación manual alternativa a CLIP: búsqueda de una serie en grilla,
  selección de su número y asociación directa del archivo físico.
- Búsqueda incremental en la barra de título al comenzar a escribir.
- Portadas completas sin recorte, conservando su relación de aspecto.
- Actualización automática de tareas en segundo plano.

El lector es un binario aparte, `../cosmic-reader`, que se lanza como proceso
externo. Se busca en `BABELCOMICS_READER`, junto al ejecutable, en
`../cosmic-reader/target/{release,debug}` o en el `PATH`.

## Ejecutar

Desde esta carpeta:

```bash
cargo run --release
```

La primera compilación descarga libcosmic y sus dependencias. La revisión de
libcosmic está fijada en `Cargo.toml` para mantener builds reproducibles.
