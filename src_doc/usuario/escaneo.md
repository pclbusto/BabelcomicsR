# Escaneo de Cómics

El proceso de escaneo es el corazón de **BabelcomicsR**. Es el mecanismo que transforma tus carpetas llenas de archivos sueltos en una biblioteca organizada, visual y navegable.

## ¿Cómo Funciona el Escaneo?

Cuando inicias un escaneo (ya sea manual o automático), la aplicación ejecuta una serie de pasos optimizados en segundo plano:

### 1. Descubrimiento de Archivos
La aplicación recorre recursivamente los directorios que hayas configurado en **Preferencias**. Actualmente, BabelcomicsR identifica los siguientes formatos:
- **.cbz:** Archivos comprimidos en formato ZIP.
- **.cbr:** Archivos comprimidos en formato RAR.

### 2. Identificación y Metadatos Locales
Para cada archivo encontrado, el sistema:
- Extrae el nombre del archivo para generar un título provisional.
- Identifica el número de issue (si está presente en el nombre).
- Calcula un "hash" único de la portada para evitar duplicados y gestionar la caché de miniaturas de forma eficiente.

### 3. Generación de Miniaturas (Thumbnails)
Este es el paso más intensivo visualmente. BabelcomicsR extrae la primera página de cada cómic para crear la miniatura que verás en la biblioteca. 
- **Optimización:** Si la miniatura ya existe en la caché local, el sistema la salta para ahorrar tiempo y CPU.
- **Asincronía:** Las miniaturas se generan en paralelo utilizando múltiples hilos (workers), aprovechando toda la potencia de tu procesador.

## Estados del Escaneo

Durante el proceso, verás diferentes indicadores en la interfaz:
- **Buscando archivos:** El sistema está mapeando tus carpetas.
- **Procesando [X/Y]:** Se están extrayendo portadas y generando metadatos.
- **Finalizado:** Tu biblioteca está lista para ser explorada.

## Consejos para una Mejor Organización

Para que el escaneo sea lo más preciso posible, te recomendamos seguir estas pautas:
1.  **Nomenclatura Clara:** Nombra tus archivos siguiendo un patrón estándar, por ejemplo: `Action Comics 483.cbz`.
2.  **Estructura de Carpetas:** Aunque BabelcomicsR puede leer carpetas desordenadas, mantener una estructura de `Editorial/Serie/Cómic.cbz` ayuda a la clasificación posterior.
3. **Caché de Thumbnails:** 
    - **Primer Escaneo:** Es un proceso intensivo que pone a prueba tu hardware. La **CPU** trabaja al máximo descomprimiendo y reescalando imágenes, mientras que el **disco** realiza miles de lecturas y escrituras. En bibliotecas masivas (ej. +8,000 ejemplares), este proceso puede durar entre 45 y 60 minutos en un SSD moderno, o más si los archivos están en un HDD o red.
    - **Escaneos Posteriores:** Una vez generada la miniatura, el sistema la reconoce instantáneamente mediante un hash de archivo. Los escaneos de mantenimiento son extremadamente rápidos (segundos o pocos minutos) ya que solo se procesan las novedades.

## Resolución de Problemas

Si algún cómic no aparece después del escaneo:
- Verifica que el archivo no esté corrupto y se pueda abrir con un visor de imágenes estándar.
- Asegúrate de que la extensión sea `.cbz` o `.cbr` (en minúsculas o mayúsculas).
- Comprueba el log de errores en la sección de **Estadísticas** si el escaneo se detuvo inesperadamente.
