# Primeros Pasos

Al iniciar **BabelcomicsR**, el objetivo es dejar configurada la biblioteca local y los servicios externos necesarios para catalogar cómics.

## Configuración Inicial

1. Abrir las preferencias de la aplicación.
2. Seleccionar la carpeta principal donde están los archivos `.cbz` o `.cbr`.
3. Configurar la API key de ComicVine si se va a usar catalogación automática.
4. Ejecutar el primer escaneo de biblioteca.

## Flujo Básico

El flujo normal de uso es:

1. Escanear la biblioteca local.
2. Revisar los cómics detectados.
3. Buscar o descargar metadatos desde ComicVine.
4. Generar miniaturas y embeddings CLIP cuando corresponda.
5. Usar las sugerencias para vincular archivos locales con números catalogados.

## Actividades

Las tareas largas aparecen en la vista de actividades: descargas, generación de miniaturas, indexación CLIP y otros procesos en segundo plano.

Cada actividad muestra su estado, progreso y resumen final para poder verificar qué se procesó.
