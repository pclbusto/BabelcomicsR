# Sistema de Thumbnails Asíncronos

Una biblioteca de cómics es, ante todo, una experiencia visual. Sin embargo, cargar y decodificar cientos de portadas simultáneamente puede bloquear cualquier interfaz de usuario. En **BabelcomicsR**, hemos diseñado un sistema de carga asíncrona que garantiza una navegación fluida incluso con miles de ejemplares.

## El Problema del Bloqueo de la UI

En aplicaciones tradicionales (especialmente en lenguajes interpretados), cargar una imagen desde el disco implica:
1.  **Lectura de Archivo (I/O):** Bloquea el hilo mientras se leen los bytes.
2.  **Decodificación (CPU):** Proceso intensivo que puede durar varios milisegundos por imagen.

Si se intentan cargar 50 portadas en el hilo principal de GTK, la interfaz se "congela" durante varias décimas de segundo, creando una experiencia de usuario pobre (lag).

## La Solución: Canales y Tokio

Nuestro sistema separa radicalmente la obtención de datos de su visualización mediante una arquitectura de **Productores y Consumidores**:

### 1. Los Productores (Tokio Workers)
Cuando la cuadrícula detecta que necesita mostrar un cómic, lanza una tarea de **Tokio** en segundo plano. Esta tarea:
- Busca la portada en la caché o la extrae del archivo original (`.cbz`/`.cbr`).
- Lee los bytes del disco.
- Envía los bytes crudos a través de un canal **`mpsc`** (Multi-Producer, Single-Consumer).

### 2. El Consumidor (Hilo GTK)
En el hilo principal de la interfaz, un temporizador optimizado (`glib::timeout_add_local`) "escucha" el canal cada 16ms (equivalente a 60 FPS). Su lógica es la siguiente:
- **Drenaje Controlado:** Procesa un máximo de 32 imágenes por "tick" (`MAX_PER_TICK`). Esto asegura que siempre haya tiempo suficiente en el ciclo de vida del frame para procesar clics y desplazamientos.
- **Decodificación Ultrarrápida:** Utilizamos `gdk::Texture::from_bytes`, que aprovecha **libjpeg-turbo** internamente para decodificar un thumbnail en menos de 0.5ms.

## Ventajas Técnicas

- **Carga On-Demand:** Solo se cargan las portadas que están a punto de entrar en el área visible.
- **Sin Bloqueo:** El I/O pesado ocurre en hilos de sistema, mientras que el hilo de GTK solo se encarga de subir los píxeles a la memoria de video.
- **Gestión de Memoria:** Usamos `WeakRef` para evitar que las tareas en segundo plano mantengan vivos elementos de la UI que el usuario ya ha cerrado o desplazado fuera de la vista.

## Resumen del Flujo de Datos

1.  **UI:** "Necesito la portada del cómic #123".
2.  **Tokio Task:** "Aquí están los bytes de la portada #123 (leídos en 5ms)".
3.  **MPSC Channel:** Envía los bytes al hilo principal.
4.  **GTK Consumer:** "Recibido. Decodifico y actualizo la tarjeta #123".
