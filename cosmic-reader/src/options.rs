//! Opciones de lectura recibidas por línea de comandos.
//!
//! Los valores coinciden con los que Babelcomics guarda en la tabla `setups`
//! (`reader_fit`, `reader_direction`, `reader_guided`, `reader_filter`), así el
//! lector no necesita acceder a la base de datos.

use image::imageops::FilterType;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Fit {
    /// La página completa entra en la ventana.
    #[default]
    Page,
    /// La página ocupa el alto de la ventana y se desplaza horizontalmente.
    Height,
    /// La página ocupa el ancho de la ventana y se desplaza verticalmente.
    Width,
    /// Tamaño real de la imagen, con desplazamiento en ambos ejes.
    Original,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Guided {
    /// Pregunta si se quiere usar la lectura guiada.
    #[default]
    Ask,
    Always,
    Never,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScaleFilter {
    Nearest,
    Triangle,
    CatmullRom,
    #[default]
    Lanczos3,
}

impl ScaleFilter {
    pub fn filter_type(self) -> FilterType {
        match self {
            Self::Nearest => FilterType::Nearest,
            Self::Triangle => FilterType::Triangle,
            Self::CatmullRom => FilterType::CatmullRom,
            Self::Lanczos3 => FilterType::Lanczos3,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReaderOptions {
    pub fit: Fit,
    /// Lectura de derecha a izquierda (manga): las flechas se invierten.
    pub right_to_left: bool,
    pub guided: Guided,
    pub filter: ScaleFilter,
}

impl ReaderOptions {
    /// Aplica una opción `--nombre valor`. Devuelve `false` si la opción no es
    /// del lector. Los valores desconocidos se ignoran y quedan los predeterminados.
    pub fn apply(&mut self, name: &str, value: &str) -> bool {
        match name {
            "--fit" => {
                self.fit = match value {
                    "height" => Fit::Height,
                    "width" => Fit::Width,
                    "original" => Fit::Original,
                    _ => Fit::Page,
                }
            }
            "--direction" => self.right_to_left = value == "rtl",
            "--guided" => {
                self.guided = match value {
                    "always" => Guided::Always,
                    "never" => Guided::Never,
                    _ => Guided::Ask,
                }
            }
            "--filter" => {
                self.filter = match value {
                    "0" | "nearest" => ScaleFilter::Nearest,
                    "1" | "triangle" => ScaleFilter::Triangle,
                    "2" | "catmullrom" => ScaleFilter::CatmullRom,
                    _ => ScaleFilter::Lanczos3,
                }
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_values_sent_by_babelcomics() {
        let mut options = ReaderOptions::default();
        assert!(options.apply("--fit", "width"));
        assert!(options.apply("--direction", "rtl"));
        assert!(options.apply("--guided", "never"));
        assert!(options.apply("--filter", "0"));
        assert_eq!(
            options,
            ReaderOptions {
                fit: Fit::Width,
                right_to_left: true,
                guided: Guided::Never,
                filter: ScaleFilter::Nearest,
            }
        );
    }

    #[test]
    fn unknown_values_fall_back_to_defaults() {
        let mut options = ReaderOptions::default();
        options.apply("--fit", "zoom");
        options.apply("--filter", "99");
        assert_eq!(options, ReaderOptions::default());
        assert!(!options.apply("--other", "x"));
    }
}
