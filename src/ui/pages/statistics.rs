use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{self as gtk};
use libadwaita as adw;
use sqlx::SqlitePool;

use babelcomics_core::repositories::{ComicbookRepository, PublisherRepository, VolumeRepository};
use crate::ui::run_in_background;

pub fn build_popover(pool: SqlitePool) -> gtk::Popover {
    let popover = gtk::Popover::new();

    let main_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .width_request(300)
        .build();

    let stack = gtk::Stack::new();
    let switcher = gtk::StackSwitcher::builder()
        .stack(&stack)
        .halign(gtk::Align::Center)
        .build();

    main_box.append(&switcher);
    main_box.append(&stack);

    // --- Secciones ---
    let comics_box = build_stats_list();
    let total_c = create_stat_row("Total Comics", "0", "image-x-generic-symbolic");
    let cat_c = create_stat_row("Catalogados", "0", "view-reveal-symbolic");
    let uncat_c = create_stat_row("Sin catalogar", "0", "view-conceal-symbolic");
    let err_c = create_stat_row("Con errores", "0", "dialog-warning-symbolic");
    err_c.add_css_class("error");
    let nothumb_c = create_stat_row("Sin thumbnail", "0", "image-missing-symbolic");
    comics_box.append(&total_c);
    comics_box.append(&cat_c);
    comics_box.append(&uncat_c);
    comics_box.append(&err_c);
    comics_box.append(&nothumb_c);
    stack.add_titled(&comics_box, Some("comics"), "Comics");

    let volumes_box = build_stats_list();
    let total_v = create_stat_row("Total Series", "0", "image-x-generic-symbolic");
    let comp_v = create_stat_row("Completadas", "0", "emblem-favorite-symbolic");
    volumes_box.append(&total_v);
    volumes_box.append(&comp_v);
    stack.add_titled(&volumes_box, Some("volumes"), "Series");

    let publishers_box = build_stats_list();
    let total_p = create_stat_row("Total Editoriales", "0", "building-symbolic");
    publishers_box.append(&total_p);
    stack.add_titled(&publishers_box, Some("publishers"), "Editoriales");

    popover.set_child(Some(&main_box));

    // Carga de datos inicial y cada vez que el popover se hace visible
    let p = pool.clone();
    let rows = (
        total_c.clone(),
        cat_c.clone(),
        uncat_c.clone(),
        err_c.clone(),
        nothumb_c.clone(),
        total_v.clone(),
        comp_v.clone(),
        total_p.clone(),
    );

    type StatRows = (
        adw::ActionRow,
        adw::ActionRow,
        adw::ActionRow,
        adw::ActionRow,
        adw::ActionRow,
        adw::ActionRow,
        adw::ActionRow,
        adw::ActionRow,
    );

    let update_stats = move |p_pool: SqlitePool, p_rows: StatRows| {
        let (tc_row, cc_row, uc_row, ec_row, nt_row, tv_row, cv_row, tp_row) = p_rows;
        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                let repo_c = ComicbookRepository::new(&p_pool);
                let repo_v = VolumeRepository::new(&p_pool);
                let repo_p = PublisherRepository::new(&p_pool);

                let t_c = repo_c.count().await.unwrap_or(0);
                let u_c = repo_c.count_uncatalogued().await.unwrap_or(0);
                let e_c = repo_c.count_with_errors().await.unwrap_or(0);
                let nt_c = repo_c.count_without_thumbnail().await.unwrap_or(0);
                let t_v = repo_v.count().await.unwrap_or(0);
                let c_v = repo_v.count_completed().await.unwrap_or(0);
                let t_p = repo_p.count().await.unwrap_or(0);

                (t_c, t_c - u_c, u_c, e_c, nt_c, t_v, c_v, t_p)
            },
            move |(tc, cc, uc, ec, nt, tv, cv, tp)| {
                tc_row.set_subtitle(&tc.to_string());
                cc_row.set_subtitle(&cc.to_string());
                uc_row.set_subtitle(&uc.to_string());
                ec_row.set_subtitle(&ec.to_string());
                nt_row.set_subtitle(&nt.to_string());
                tv_row.set_subtitle(&tv.to_string());
                cv_row.set_subtitle(&cv.to_string());
                tp_row.set_subtitle(&tp.to_string());
            },
        );
    };

    let update_stats_initial = update_stats.clone();
    let p_initial = p.clone();
    let rows_initial = rows.clone();

    // Al abrir el popover, refrescar datos
    popover.connect_visible_notify(move |pop| {
        if pop.get_visible() {
            update_stats(p.clone(), rows.clone());
        }
    });

    // Carga inicial al construir (opcional, pero ayuda a que no aparezca en cero al primer click)
    update_stats_initial(p_initial, rows_initial);

    popover
}

fn build_stats_list() -> gtk::ListBox {
    gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build()
}

fn create_stat_row(title: &str, initial_value: &str, icon_name: &str) -> adw::ActionRow {
    let row = adw::ActionRow::builder()
        .title(title)
        .subtitle(initial_value)
        .build();

    row.add_prefix(
        &gtk::Image::builder()
            .icon_name(icon_name)
            .pixel_size(16)
            .build(),
    );

    row
}
