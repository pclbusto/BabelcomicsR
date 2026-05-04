# Clasificación de Cómics

La clasificación relaciona archivos locales con metadatos estructurados de series, volúmenes y números.

## Datos Locales

El escaneo local registra:

- Ruta del archivo.
- Título detectado.
- Número inferido cuando es posible.
- Miniatura extraída desde la portada.
- Estado de procesamiento.

## Metadatos

Los metadatos catalogados pueden venir de ComicVine e incluyen:

- Editorial.
- Volumen o serie.
- Número.
- Título.
- Descripción.
- Portadas candidatas.

## Sugerencias

BabelcomicsR puede comparar portadas locales contra portadas catalogadas usando embeddings CLIP. Esto ayuda a encontrar coincidencias visuales incluso cuando el nombre del archivo no es confiable.

## Resultado

Cuando una sugerencia se acepta, el cómic local queda vinculado al número catalogado y puede mostrar información enriquecida en la interfaz.
