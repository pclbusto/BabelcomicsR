# ComicVine API

BabelcomicsR usa ComicVine para descargar información de editoriales, volúmenes, números y portadas.

## Configuración

Para usar la integración se necesita una API key de ComicVine. La clave se configura desde las preferencias de la aplicación.

La aplicación guarda la configuración localmente y la usa para consultar la API cuando se realizan búsquedas o descargas de metadatos.

## Descarga de Volúmenes

Al descargar un volumen, BabelcomicsR:

1. Obtiene los detalles del volumen.
2. Guarda o actualiza la editorial.
3. Guarda o actualiza la serie.
4. Obtiene la lista completa de números.
5. Registra portadas y metadatos de cada número.
6. Genera embeddings CLIP para las portadas descargadas si corresponde.

## Actividades

La descarga informa el progreso por número y al finalizar muestra cuántos números se procesaron sobre el total del volumen.

La indexación CLIP se muestra como una actividad separada para poder distinguir descarga de metadatos e indexación visual.
