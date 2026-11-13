mod app;

use cosmic::iced::{Limits, Size};

fn main() -> cosmic::iced::Result {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("no se pudo iniciar Tokio");

    let context = runtime
        .block_on(babelcomics_core::bootstrap::init())
        .expect("no se pudo iniciar Babelcomics");

    let settings = cosmic::app::Settings::default()
        .size(Size::new(1280.0, 820.0))
        .size_limits(Limits::NONE.min_width(720.0).min_height(480.0));

    cosmic::app::run::<app::Babelcomics>(settings, context.pool)
}
