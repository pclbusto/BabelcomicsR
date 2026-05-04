# Interfaz Gráfica (GTK4 / Libadwaita)

La interfaz de **BabelcomicsR** está construida sobre el ecosistema de **GNOME (GTK4 y Libadwaita)**, lo que le otorga una apariencia nativa y moderna en el escritorio.

## Diseño de Cards de Cómics

Una de las piezas clave de la experiencia visual es el sistema de cuadrículas de cómics. Este sistema está diseñado para ser flexible y adaptarse tanto a las preferencias del usuario como a las proporciones reales de las portadas de los cómics.

### Cuadrícula Adaptativa (WrapBox)

Se utiliza un `adw::WrapBox` para distribuir las tarjetas de forma automática. A diferencia de un grid tradicional con columnas fijas, el `WrapBox` ajusta el número de elementos en cada fila según el ancho disponible en la ventana, manteniendo un espaciado uniforme entre ellos.

### Proporción y Ancho Variable

Un cómic puede tener diferentes proporciones (retrato estándar, formatos europeos más anchos o portadas apaisadas). Nuestro sistema de tarjetas respeta estas proporciones sin dejar huecos vacíos:

1.  **Altura Fija por Configuración:** El usuario puede elegir entre tres tamaños de tarjeta (Chico, Mediano, Grande). Esta configuración define una **altura fija** para todas las portadas, asegurando filas alineadas.
2.  **Ancho Dinámico:** No forzamos un ancho fijo. El contenedor de la tarjeta se encoge o ensancha exactamente según el ancho de la imagen cargada.
3.  **Prevención de Huecos (Spacing Inconsistente):** 
    - Las tarjetas no tienen márgenes laterales manuales; el espaciado lo gestiona centralmente el `WrapBox`.
    - Se utiliza `hexpand(false)` y `halign(Center)` en los contenedores para evitar que el texto o los datos ensanchen la tarjeta más allá de lo necesario.
    - El título se envuelve automáticamente en un máximo de dos líneas con un límite estricto de caracteres (`max_width_chars`) para evitar deformar la proporción visual de la tarjeta.

### Ejemplo de Implementación (Rust)

```rust,ignore
// Ejemplo simplificado de creación de tarjeta
let card = gtk::Box::builder()
    .orientation(gtk::Orientation::Vertical)
    .hexpand(false)
    .halign(gtk::Align::Center)
    .css_classes(["card"])
    .build();

let image_container = gtk::Box::builder()
    .height_request(ch_configurada)
    .hexpand(false)
    .halign(gtk::Align::Center)
    .build();
```

Este enfoque asegura que una colección con portadas de diferentes orígenes siempre se vea visualmente equilibrada y densa, sin "aire" desperdiciado.
