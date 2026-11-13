use std::cell::RefCell;
use std::future::Future;
use std::rc::Rc;

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{self as gtk, gio};
use libadwaita as adw;
use sqlx::SqlitePool;

use crate::ui;
use babelcomics_core::helpers::{
    comicvine_client::ComicVineClient,
    paths, scan_service,
    thumbnail::{CardSize, ReaderFilter},
};
use babelcomics_core::repositories::SetupRepository;

// ---------------------------------------------------------------------------
// Diálogo de preferencias
// ---------------------------------------------------------------------------

pub fn build_preferences_dialog(pool: SqlitePool) -> adw::PreferencesDialog {
    let dialog = adw::PreferencesDialog::new();
    dialog.set_title("Preferencias");

    // ── PÁGINA 1: GENERAL ────────────────────────────────────────────────────
    let page_general = adw::PreferencesPage::builder()
        .title("General")
        .icon_name("preferences-system-symbolic")
        .build();

    // Grupo: ComicVine API
    let group_api = adw::PreferencesGroup::builder()
        .title("ComicVine API")
        .build();

    let row_api_url = adw::EntryRow::builder().title("API URL").build();
    group_api.add(&row_api_url);

    let row_api_key = adw::PasswordEntryRow::builder().title("API Key").build();
    group_api.add(&row_api_key);

    let row_validar = adw::ActionRow::builder()
        .title("Validar conexión")
        .subtitle("Comprueba que la API Key funciona")
        .build();
    let btn_validar = gtk::Button::builder()
        .label("Probar")
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action"])
        .build();
    row_validar.add_suffix(&btn_validar);
    row_validar.set_activatable_widget(Some(&btn_validar));
    group_api.add(&row_validar);

    // Conectar botón Probar
    {
        let row_url = row_api_url.clone();
        let row_key = row_api_key.clone();
        let btn_v = btn_validar.clone();
        let row_v = row_validar.clone();

        btn_validar.connect_clicked(move |_| {
            let api_key = row_key.text().to_string();
            let api_url = row_url.text().to_string();

            if api_key.is_empty() {
                row_v.set_subtitle("❌ Introduce una API Key");
                return;
            }

            btn_v.set_sensitive(false);
            row_v.set_subtitle("⌛ Probando conexión…");

            let btn_done = btn_v.clone();
            let row_done = row_v.clone();

            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    let url_opt = if api_url.is_empty() {
                        None
                    } else {
                        Some(api_url)
                    };
                    let client =
                        ComicVineClient::new(api_key, url_opt).map_err(|e| e.to_string())?;
                    client.validate().await.map_err(|e| e.to_string())
                },
                move |result| {
                    match result {
                        Ok(_) => row_done.set_subtitle("✅ Conexión establecida correctamente"),
                        Err(e) => row_done.set_subtitle(&format!("❌ Error: {}", e)),
                    }
                    btn_done.set_sensitive(true);
                },
            );
        });
    }

    let row_rate = adw::SpinRow::builder()
        .title("Intervalo entre requests")
        .subtitle("Segundos")
        .digits(1)
        .build();
    row_rate.set_adjustment(Some(&gtk::Adjustment::new(0.5, 0.1, 5.0, 0.1, 0.5, 0.0)));
    group_api.add(&row_rate);

    // Cargar valores iniciales de la API y luego conectar eventos de guardado
    {
        let pool_api = pool.clone();
        let pool_save = pool.clone();
        let row_url = row_api_url.clone();
        let row_key = row_api_key.clone();
        let row_r = row_rate.clone();

        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                SetupRepository::new(&pool_api)
                    .get()
                    .await
                    .unwrap_or_default()
            },
            move |setup| {
                if let Some(url) = setup.api_url {
                    row_url.set_text(&url);
                }
                if let Some(key) = setup.api_key_encrypted {
                    row_key.set_text(&key);
                }
                row_r.set_value(setup.intervalo_api);

                // Conectar señales de guardado SOLO después de cargar los valores iniciales
                let r_url = row_url.clone();
                let r_key = row_key.clone();
                let r_rate = row_r.clone();
                let p_save = pool_save.clone();

                let trigger_save = move || {
                    let pool_task = p_save.clone();
                    let url = r_url.text().to_string();
                    let key = r_key.text().to_string();
                    let interval = r_rate.value();

                    run_in_background(
                        tokio::runtime::Handle::current(),
                        async move {
                            let repo = SetupRepository::new(&pool_task);
                            let mut setup = repo.get().await.unwrap_or_default();
                            setup.api_url = Some(url);
                            setup.api_key_encrypted = if key.is_empty() { None } else { Some(key) };
                            setup.intervalo_api = interval;
                            let _ = repo.save(&setup).await;
                        },
                        |_| {},
                    );
                };

                let ts1 = Rc::new(trigger_save);
                let ts2 = ts1.clone();
                let ts3 = ts1.clone();

                row_url.connect_changed(move |_| ts1());
                row_api_key.connect_changed(move |_| ts2());
                row_r.connect_value_notify(move |_| ts3());
            },
        );
    }

    // Grupo: Directorios de escaneo
    let group_dirs = adw::PreferencesGroup::builder()
        .title("Directorios de escaneo")
        .build();

    // Estado compartido: lista de (id, row_widget) para poder limpiar la lista
    let dir_rows: Rc<RefCell<Vec<(i64, adw::ActionRow)>>> = Rc::new(RefCell::new(Vec::new()));

    // Subgrupo dinámico donde aparecen los directorios añadidos
    let group_dir_list = adw::PreferencesGroup::new();

    let row_add_dir = adw::ActionRow::builder()
        .title("Añadir directorio")
        .subtitle("Selecciona una carpeta para escanear")
        .build();
    let btn_add_dir = gtk::Button::builder()
        .icon_name("folder-new-symbolic")
        .tooltip_text("Seleccionar carpeta")
        .valign(gtk::Align::Center)
        .css_classes(["suggested-action"])
        .build();
    row_add_dir.add_suffix(&btn_add_dir);
    row_add_dir.set_activatable_widget(Some(&btn_add_dir));
    group_dirs.add(&row_add_dir);

    // Botón escanear + label de estado
    let row_scan = adw::ActionRow::builder()
        .title("Escanear directorios")
        .subtitle("Busca cómics en los directorios configurados")
        .build();
    let scan_status_label = gtk::Label::builder()
        .label("")
        .valign(gtk::Align::Center)
        .css_classes(["dim-label"])
        .build();
    let btn_scan = gtk::Button::builder()
        .label("Escanear")
        .valign(gtk::Align::Center)
        .build();
    row_scan.add_suffix(&scan_status_label);
    row_scan.add_suffix(&btn_scan);
    row_scan.set_activatable_widget(Some(&btn_scan));
    group_dirs.add(&row_scan);

    // Carga inicial de los directorios desde la BD
    {
        let group_init = group_dir_list.clone();
        let dir_rows_init = dir_rows.clone();
        let pool_ui = pool.clone();
        let pool_task = pool.clone();
        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                SetupRepository::new(&pool_task)
                    .get_directorios()
                    .await
                    .unwrap_or_default()
            },
            move |dirs| {
                populate_dir_list(&group_init, &dir_rows_init, dirs, &pool_ui);
            },
        );
    }

    // Botón "Añadir directorio"
    {
        let pool_add = pool.clone();
        let dir_rows_add = dir_rows.clone();
        let group_dir_list_add = group_dir_list.clone();
        let dialog_weak = dialog.downgrade();
        btn_add_dir.connect_clicked(move |_| {
            let file_dialog = gtk::FileDialog::new();
            file_dialog.set_title("Seleccionar directorio");
            file_dialog.set_modal(true);

            let pool_cb = pool_add.clone();
            let dir_rows_cb = dir_rows_add.clone();
            let group_cb = group_dir_list_add.clone();
            let parent: Option<gtk::Window> = dialog_weak
                .upgrade()
                .and_then(|d| d.root())
                .and_then(|r| r.downcast::<gtk::Window>().ok());

            file_dialog.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |result| {
                let Ok(file) = result else { return };
                let Some(path) = file.path() else { return };
                let path_str = path.to_string_lossy().to_string();

                let pool_task = pool_cb.clone();
                let pool_ui = pool_cb.clone();
                let dir_rows_ui = dir_rows_cb.clone();
                let group_ui = group_cb.clone();

                run_in_background(
                    tokio::runtime::Handle::current(),
                    async move {
                        let repo = SetupRepository::new(&pool_task);
                        match repo.add_directorio(&path_str).await {
                            Ok(_) => repo.get_directorios().await.unwrap_or_default(),
                            Err(e) => {
                                tracing::error!("Error añadiendo directorio: {e}");
                                repo.get_directorios().await.unwrap_or_default()
                            }
                        }
                    },
                    move |dirs| {
                        populate_dir_list(&group_ui, &dir_rows_ui, dirs, &pool_ui);
                    },
                );
            });
        });
    }

    // Botón "Escanear"
    {
        let pool_scan = pool.clone();
        let btn_scan_clone = btn_scan.clone();
        let label_clone = scan_status_label.clone();
        btn_scan.connect_clicked(move |_| {
            btn_scan_clone.set_sensitive(false);
            label_clone.set_label("Escaneando…");

            let btn_done = btn_scan_clone.clone();
            let label_done = label_clone.clone();
            let pool_task = pool_scan.clone();
            let job_id = babelcomics_core::helpers::background_jobs::start_job(
                "Escaneo de biblioteca",
                "Preparando escaneo...",
                babelcomics_core::helpers::background_jobs::JobKind::Scan,
            );
            let job_id_task = job_id.clone();
            let job_id_done = job_id.clone();

            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    let repo = SetupRepository::new(&pool_task);
                    let dirs = repo.get_directorios().await.unwrap_or_default();
                    let dir_paths: Vec<String> = dirs.into_iter().map(|d| d.path).collect();

                    if dir_paths.is_empty() {
                        return Err("Sin directorios configurados".to_string());
                    }
                    babelcomics_core::helpers::background_jobs::update_job(
                        &job_id_task,
                        format!("Escaneando {} directorios...", dir_paths.len()),
                        0.1,
                    );

                    let card_size = CardSize::from_db(
                        SetupRepository::new(&pool_task)
                            .get()
                            .await
                            .map(|s| s.thumbnail_size)
                            .unwrap_or(1),
                    );

                    scan_service::run_scan(&pool_task, &dir_paths, card_size)
                        .await
                        .map(|r| {
                            let has_errors = !r.errors.is_empty();
                            (format_scan_summary(&r), has_errors)
                        })
                        .map_err(|e| e.to_string())
                },
                move |result| {
                    match result {
                        Ok((msg, has_errors)) => {
                            label_done.set_label(&msg);
                            babelcomics_core::helpers::background_jobs::finish_job(
                                &job_id_done,
                                msg,
                                has_errors,
                            );
                        }
                        Err(e) => {
                            label_done.set_label(&e);
                            babelcomics_core::helpers::background_jobs::finish_job(
                                &job_id_done,
                                e,
                                true,
                            );
                        }
                    }
                    btn_done.set_sensitive(true);
                },
            );
        });
    }

    // Grupo: Interfaz
    let group_interfaz = adw::PreferencesGroup::builder().title("Interfaz").build();

    let row_tema = adw::ComboRow::builder().title("Tema").build();
    let temas = gtk::StringList::new(&["Seguir sistema", "Claro", "Oscuro"]);
    row_tema.set_model(Some(&temas));
    group_interfaz.add(&row_tema);

    let row_card_size = adw::ComboRow::builder()
        .title("Tamaño de miniatura")
        .subtitle("Pequeña 160×220 · Mediana 240×320 · Grande 320×430")
        .build();
    let card_size_model = gtk::StringList::new(&["Pequeña", "Mediana", "Grande"]);
    row_card_size.set_model(Some(&card_size_model));
    group_interfaz.add(&row_card_size);

    // Cargar el valor guardado en BD y aplicarlo al ComboRow
    {
        let row = row_card_size.clone();
        let pool_cs = pool.clone();
        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                SetupRepository::new(&pool_cs)
                    .get()
                    .await
                    .map(|s| s.thumbnail_size)
                    .unwrap_or(1)
            },
            move |val| {
                row.set_selected(CardSize::from_db(val).combo_index());
            },
        );
    }

    // Guardar en BD cuando el usuario cambia la selección
    {
        let pool_cs = pool.clone();
        row_card_size.connect_selected_notify(move |row| {
            let size = CardSize::from_combo_index(row.selected()).to_db();
            let pool_task = pool_cs.clone();
            run_in_background(
                tokio::runtime::Handle::current(),
                async move { SetupRepository::new(&pool_task).set_card_size(size).await },
                |result| {
                    if let Err(e) = result {
                        tracing::error!("Error guardando tamaño de miniatura: {e}");
                    }
                },
            );
        });
    }

    let row_items = adw::SpinRow::builder().title("Ítems por lote").build();
    row_items.set_adjustment(Some(&gtk::Adjustment::new(
        20.0, 10.0, 100.0, 5.0, 20.0, 0.0,
    )));
    group_interfaz.add(&row_items);

    let row_scroll_sens = adw::SpinRow::builder()
        .title("Sensibilidad de scroll")
        .digits(1)
        .build();
    row_scroll_sens.set_adjustment(Some(&gtk::Adjustment::new(1.0, 0.1, 5.0, 0.1, 0.5, 0.0)));
    group_interfaz.add(&row_scroll_sens);

    let row_scroll_cd = adw::SpinRow::builder()
        .title("Cooldown de scroll")
        .subtitle("Milisegundos")
        .build();
    row_scroll_cd.set_adjustment(Some(&gtk::Adjustment::new(
        100.0, 50.0, 1000.0, 50.0, 100.0, 0.0,
    )));
    group_interfaz.add(&row_scroll_cd);

    // Grupo: Lector
    let group_lector = adw::PreferencesGroup::builder().title("Lector").build();

    let row_reader_filter = adw::ComboRow::builder()
        .title("Algoritmo de escalado de páginas")
        .subtitle("Calidad al generar miniaturas de páginas en el lector")
        .build();
    let reader_filter_model = gtk::StringList::new(&[
        "Nearest (más rápido)",
        "Bilineal",
        "CatmullRom",
        "Lanczos3 (mejor calidad)",
    ]);
    row_reader_filter.set_model(Some(&reader_filter_model));
    group_lector.add(&row_reader_filter);

    // Cargar valor guardado en BD
    {
        let row = row_reader_filter.clone();
        let pool_rf = pool.clone();
        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                SetupRepository::new(&pool_rf)
                    .get()
                    .await
                    .map(|s| s.reader_filter)
                    .unwrap_or(3)
            },
            move |val| {
                row.set_selected(ReaderFilter::from_db(val).combo_index());
            },
        );
    }

    // Guardar en BD cuando el usuario cambia la selección
    {
        let pool_rf = pool.clone();
        row_reader_filter.connect_selected_notify(move |row| {
            let filter = ReaderFilter::from_combo_index(row.selected()).to_db();
            let pool_task = pool_rf.clone();
            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    SetupRepository::new(&pool_task)
                        .set_reader_filter(filter)
                        .await
                },
                |result| {
                    if let Err(e) = result {
                        tracing::error!("Error guardando filtro de lector: {e}");
                    }
                },
            );
        });
    }

    page_general.add(&group_api);
    page_general.add(&group_dirs);
    page_general.add(&group_dir_list);
    page_general.add(&group_interfaz);
    page_general.add(&group_lector);
    dialog.add(&page_general);

    // ── PÁGINA 2: AVANZADO ───────────────────────────────────────────────────
    let page_avanzado = adw::PreferencesPage::builder()
        .title("Avanzado")
        .icon_name("applications-engineering-symbolic")
        .build();

    // Grupo: Base de datos
    let group_db = adw::PreferencesGroup::builder()
        .title("Base de datos")
        .build();

    let row_db_info = adw::ActionRow::builder()
        .title("Base de datos")
        .subtitle("—")
        .build();
    let btn_db_open = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Abrir directorio")
        .valign(gtk::Align::Center)
        .build();
    row_db_info.add_suffix(&btn_db_open);
    group_db.add(&row_db_info);

    let row_backup = adw::ActionRow::builder()
        .title("Backup de base de datos")
        .subtitle("Crea una copia de seguridad con marca de tiempo")
        .build();
    let btn_backup = gtk::Button::builder()
        .label("Crear Backup")
        .valign(gtk::Align::Center)
        .build();
    row_backup.add_suffix(&btn_backup);
    row_backup.set_activatable_widget(Some(&btn_backup));
    group_db.add(&row_backup);

    let row_vacuum = adw::ActionRow::builder()
        .title("Optimizar base de datos")
        .subtitle("Ejecuta VACUUM para reducir tamaño y mejorar rendimiento")
        .build();
    let btn_vacuum = gtk::Button::builder()
        .label("Optimizar")
        .valign(gtk::Align::Center)
        .build();
    row_vacuum.add_suffix(&btn_vacuum);
    row_vacuum.set_activatable_widget(Some(&btn_vacuum));
    group_db.add(&row_vacuum);

    // Grupo: Rendimiento
    let group_perf = adw::PreferencesGroup::builder()
        .title("Rendimiento")
        .build();

    let row_workers = adw::SpinRow::builder()
        .title("Workers concurrentes")
        .subtitle("Para escaneo y extracción")
        .build();
    row_workers.set_adjustment(Some(&gtk::Adjustment::new(5.0, 1.0, 20.0, 1.0, 5.0, 0.0)));
    group_perf.add(&row_workers);

    let row_cache = adw::SwitchRow::builder()
        .title("Cache de miniaturas")
        .subtitle("Mantener miniaturas en memoria")
        .build();
    row_cache.set_active(true);
    group_perf.add(&row_cache);

    let row_cleanup = adw::SwitchRow::builder()
        .title("Limpieza automática")
        .subtitle("Limpiar archivos temporales al cerrar")
        .build();
    row_cleanup.set_active(true);
    group_perf.add(&row_cleanup);

    // Grupo: Miniaturas
    let group_thumbs_adv = adw::PreferencesGroup::builder().title("Miniaturas").build();

    // Carpeta de thumbnails
    let row_thumb_dir = adw::ActionRow::builder()
        .title("Carpeta de thumbnails")
        .subtitle(paths::thumbnails_dir().to_string_lossy().as_ref())
        .build();
    let btn_thumb_sel = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Seleccionar carpeta")
        .valign(gtk::Align::Center)
        .build();
    let btn_thumb_clear = gtk::Button::builder()
        .icon_name("edit-clear-symbolic")
        .tooltip_text("Restaurar carpeta por defecto")
        .valign(gtk::Align::Center)
        .build();
    row_thumb_dir.add_suffix(&btn_thumb_sel);
    row_thumb_dir.add_suffix(&btn_thumb_clear);
    group_thumbs_adv.add(&row_thumb_dir);

    // Botón seleccionar carpeta de thumbnails
    {
        let pool_t = pool.clone();
        let row_t = row_thumb_dir.clone();
        let dialog_weak = dialog.downgrade();
        btn_thumb_sel.connect_clicked(move |_| {
            let file_dialog = gtk::FileDialog::new();
            file_dialog.set_title("Seleccionar carpeta de thumbnails");
            file_dialog.set_modal(true);

            let pool_cb = pool_t.clone();
            let row_cb = row_t.clone();
            let parent: Option<gtk::Window> = dialog_weak
                .upgrade()
                .and_then(|d| d.root())
                .and_then(|r| r.downcast::<gtk::Window>().ok());
            let parent_for_closure = parent.clone();

            file_dialog.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |result| {
                let Ok(file) = result else { return };
                let Some(new_path) = file.path() else { return };
                let new_path_str = new_path.to_string_lossy().to_string();

                let old_path = paths::thumbnails_dir();
                let pool_task = pool_cb.clone();
                let row_cb_clone = row_cb.clone();
                let parent_win = parent_for_closure.clone();

                // Diálogo moderno de confirmación (Libadwaita 1.6+)
                let dialog = adw::AlertDialog::builder()
                    .heading("¿Migrar thumbnails actuales?")
                    .body("¿Quieres mover tus thumbnails actuales a la nueva ubicación? Esto evitará tener que regenerarlos todos.")
                    .build();

                dialog.add_response("cancel", "Cancelar");
                dialog.add_response("solo_cambiar", "Solo cambiar ruta");
                dialog.add_response("migrar", "Migrar archivos");

                dialog.set_response_appearance("solo_cambiar", adw::ResponseAppearance::Destructive);
                dialog.set_response_appearance("migrar", adw::ResponseAppearance::Suggested);

                dialog.set_default_response(Some("migrar"));
                dialog.set_close_response("cancel");

                // Seguro: estamos dentro del callback de select_folder, la ventana padre existe
                let parent_widget: &gtk::Widget = parent_win.as_ref().unwrap().upcast_ref();
                dialog.choose(parent_widget, None::<&gio::Cancellable>, move |response| {
                    if response == "cancel" {
                        return;
                    }

                    let should_migrate = response == "migrar";
                    let old_p = old_path.clone();
                    let new_p = new_path.clone();
                    let new_p_str = new_path_str.clone();
                    let p_task = pool_task.clone();
                    let r_cb = row_cb_clone.clone();

                    run_in_background(
                        tokio::runtime::Handle::current(),
                        async move {
                            if should_migrate {
                                if let Err(e) = paths::migrate_thumbnails(old_p, new_p.clone()) {
                                    tracing::error!("Error migrando thumbnails: {e}");
                                    return Err(format!("Error al migrar thumbnails: {e}"));
                                }
                            }

                            paths::set_thumbnails_base(new_p);

                            let repo = SetupRepository::new(&p_task);
                            if let Err(e) = repo.set_carpeta_thumbnails(Some(&new_p_str)).await {
                                tracing::error!("Error guardando carpeta de thumbnails: {e}");
                                return Err(e.to_string());
                            }
                            Ok(new_p_str)
                        },
                        move |res| {
                            if let Ok(path) = res {
                                r_cb.set_subtitle(&path);
                            }
                        }
                    );
                });
            });
        });
    }

    // Botón restaurar carpeta por defecto
    {
        let pool_t = pool.clone();
        let row_t = row_thumb_dir.clone();
        btn_thumb_clear.connect_clicked(move |_| {
            paths::initialize_thumbnails_base(None);
            let default_path = paths::thumbnails_dir().to_string_lossy().to_string();
            row_t.set_subtitle(&default_path);

            let pool_task = pool_t.clone();
            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    SetupRepository::new(&pool_task)
                        .set_carpeta_thumbnails(None)
                        .await
                },
                |result| {
                    if let Err(e) = result {
                        tracing::error!("Error restaurando carpeta de thumbnails: {e}");
                    }
                },
            );
        });
    }

    let row_regen = adw::ActionRow::builder()
        .title("Regenerar covers de volúmenes")
        .subtitle("Descarga los covers desde ComicVine nuevamente")
        .build();
    let btn_regen = gtk::Button::builder()
        .label("Regenerar")
        .valign(gtk::Align::Center)
        .css_classes(["destructive-action"])
        .build();
    row_regen.add_suffix(&btn_regen);
    row_regen.set_activatable_widget(Some(&btn_regen));
    group_thumbs_adv.add(&row_regen);

    let row_clear_cache = adw::ActionRow::builder()
        .title("Limpiar cache de miniaturas")
        .subtitle("Elimina todos los thumbnails locales")
        .build();
    let btn_clear_cache = gtk::Button::builder()
        .label("Limpiar")
        .valign(gtk::Align::Center)
        .css_classes(["destructive-action"])
        .build();
    row_clear_cache.add_suffix(&btn_clear_cache);
    row_clear_cache.set_activatable_widget(Some(&btn_clear_cache));
    group_thumbs_adv.add(&row_clear_cache);

    let row_stats = adw::ActionRow::builder()
        .title("Estadísticas de miniaturas")
        .subtitle("—")
        .build();
    let btn_refresh_stats = gtk::Button::builder()
        .icon_name("view-refresh-symbolic")
        .tooltip_text("Actualizar estadísticas")
        .valign(gtk::Align::Center)
        .build();
    row_stats.add_suffix(&btn_refresh_stats);
    group_thumbs_adv.add(&row_stats);

    // Grupo: Portadas y similitud
    let group_covers = adw::PreferencesGroup::builder()
        .title("Portadas y similitud")
        .description("Hashes perceptuales usados para sugerir catalogación automática")
        .build();

    let row_rehash = adw::ActionRow::builder()
        .title("Recalcular hashes de portadas")
        .subtitle("Listo")
        .build();
    let btn_rehash = gtk::Button::builder()
        .label("Recalcular")
        .valign(gtk::Align::Center)
        .build();
    row_rehash.add_suffix(&btn_rehash);
    row_rehash.set_activatable_widget(Some(&btn_rehash));
    group_covers.add(&row_rehash);

    {
        let pool_rh = pool.clone();
        let row_rh = row_rehash.clone();
        let btn_rh = btn_rehash.clone();
        btn_rehash.connect_clicked(move |_| {
            btn_rh.set_sensitive(false);
            row_rh.set_subtitle("⌛ Calculando…");

            let pool_task = pool_rh.clone();
            let row_done = row_rh.clone();
            let btn_done = btn_rh.clone();

            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    use sqlx::Row;

                    // Limpiar embeddings anteriores
                    let _ = sqlx::query(
                        "UPDATE comicbooks SET embedding = NULL WHERE id_comicbook_info IS NOT NULL"
                    )
                    .execute(&pool_task)
                    .await;

                    // Obtener todos los comics catalogados con su path
                    let rows = sqlx::query(
                        "SELECT id_comicbook, path FROM comicbooks \
                         WHERE id_comicbook_info IS NOT NULL AND en_papelera = 0",
                    )
                    .fetch_all(&pool_task)
                    .await
                    .unwrap_or_default();

                    let entries: Vec<(i64, String)> =
                        rows.iter().map(|r| (r.get(0), r.get(1))).collect();

                    // Calcular hashes a partir del thumbnail existente (o extraer si no existe)
                    let card_size = babelcomics_core::helpers::thumbnail::CardSize::from_db(
                        SetupRepository::new(&pool_task)
                            .get()
                            .await
                            .unwrap_or_default()
                            .thumbnail_size,
                    );

                    // Procesamiento secuencial: un archivo a la vez para no saturar la RAM.
                    // Para comics sin thumbnail se extrae solo la portada (no el archivo completo).
                    let computed = tokio::task::spawn_blocking(move || {
                        let mut out = Vec::new();
                        for (id, path) in &entries {
                            let thumb = babelcomics_core::helpers::paths::comic_thumbnail_path(
                                *id, card_size,
                            );
                            let bytes = if thumb.exists() {
                                match std::fs::read(&thumb) {
                                    Ok(b) => b,
                                    Err(_) => continue,
                                }
                            } else {
                                match babelcomics_core::helpers::extractor::extract_cover(path) {
                                    Ok(b) => b,
                                    Err(_) => continue,
                                }
                            };
                            if let Some(hash) =
                                babelcomics_core::helpers::cover_hash::compute_hash(&bytes)
                            {
                                out.push((*id, hash));
                            }
                        }
                        out
                    })
                    .await
                    .unwrap_or_default();

                    let total = computed.len();
                    for (id, hash) in computed {
                        let _ = sqlx::query(
                            "UPDATE comicbooks SET embedding = ? WHERE id_comicbook = ?",
                        )
                        .bind(&hash)
                        .bind(id)
                        .execute(&pool_task)
                        .await;
                    }

                    Ok::<_, sqlx::Error>(format!("✅ {} comics procesados", total))
                },
                move |result| {
                    btn_done.set_sensitive(true);
                    match result {
                        Ok(msg) => row_done.set_subtitle(&msg),
                        Err(e) => row_done.set_subtitle(&format!("❌ Error: {}", e)),
                    }
                },
            );
        });
    }

    let row_clip_boot = adw::SwitchRow::builder()
        .title("Generar embeddings CLIP al arrancar")
        .subtitle("Analiza portadas nuevas al abrir la aplicación (CPU intensivo)")
        .build();
    group_covers.add(&row_clip_boot);

    // Cargar valor inicial
    {
        let pool_ui = pool.clone();
        let row = row_clip_boot.clone();
        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                SetupRepository::new(&pool_ui)
                    .get()
                    .await
                    .map(|s| s.clip_al_arranque)
                    .unwrap_or(true)
            },
            move |val| {
                row.set_active(val);
            },
        );
    }

    // Guardar cambio
    {
        let pool_task = pool.clone();
        row_clip_boot.connect_active_notify(move |row| {
            let val = row.is_active();
            let p = pool_task.clone();
            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    let repo = SetupRepository::new(&p);
                    if let Ok(mut setup) = repo.get().await {
                        setup.clip_al_arranque = val;
                        let _ = repo.save(&setup).await;
                    }
                },
                |_| {},
            );
        });
    }

    page_avanzado.add(&group_db);
    page_avanzado.add(&group_perf);
    page_avanzado.add(&group_thumbs_adv);
    page_avanzado.add(&group_covers);
    dialog.add(&page_avanzado);

    dialog
}

fn format_scan_summary(result: &scan_service::ScanResult) -> String {
    let mut parts = vec![
        format!("{} cómics encontrados", result.total_found),
        format!("{} nuevos", result.new_inserted),
        format!("{} miniaturas generadas", result.covers_generated),
    ];
    if !result.errors.is_empty() {
        parts.push(format!("{} errores", result.errors.len()));
    }
    parts.join(" · ")
}

// ---------------------------------------------------------------------------
// Lista dinámica de directorios
// ---------------------------------------------------------------------------

/// Limpia y repobla el grupo con los directorios actuales de la BD.
fn populate_dir_list(
    group: &adw::PreferencesGroup,
    dir_rows: &Rc<RefCell<Vec<(i64, adw::ActionRow)>>>,
    dirs: Vec<babelcomics_core::models::SetupDirectorio>,
    pool: &SqlitePool,
) {
    for (_, row) in dir_rows.borrow().iter() {
        group.remove(row);
    }
    dir_rows.borrow_mut().clear();

    for dir in dirs {
        let path = std::path::Path::new(&dir.path);
        let exists = path.exists();

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| dir.path.clone());

        let subtitle = if exists {
            dir.path.clone()
        } else {
            format!("❌ Directorio no encontrado: {}", dir.path)
        };

        let row = adw::ActionRow::builder()
            .title(&name)
            .subtitle(&subtitle)
            .build();

        if exists {
            let path_open = dir.path.clone();
            let btn_open = gtk::Button::builder()
                .icon_name("folder-open-symbolic")
                .tooltip_text("Abrir en explorador")
                .valign(gtk::Align::Center)
                .build();
            btn_open.connect_clicked(move |_| {
                if let Err(e) = gio::AppInfo::launch_default_for_uri(
                    &format!("file://{}", path_open),
                    gio::AppLaunchContext::NONE,
                ) {
                    tracing::warn!("No se pudo abrir el directorio: {e}");
                }
            });
            row.add_suffix(&btn_open);
        }

        let btn_del = gtk::Button::builder()
            .icon_name("user-trash-symbolic")
            .tooltip_text("Eliminar directorio")
            .valign(gtk::Align::Center)
            .css_classes(["destructive-action"])
            .build();
        {
            let pool_task = pool.clone();
            let pool_ui = pool.clone();
            let dir_id = dir.id;
            let dir_rows_ui = dir_rows.clone();
            let group_ui = group.clone();
            btn_del.connect_clicked(move |_| {
                let pool_task = pool_task.clone();
                let pool_ui = pool_ui.clone();
                let dir_rows_ui = dir_rows_ui.clone();
                let group_ui = group_ui.clone();
                run_in_background(
                    tokio::runtime::Handle::current(),
                    async move {
                        let repo = SetupRepository::new(&pool_task);
                        let _ = repo.remove_directorio(dir_id).await;
                        repo.get_directorios().await.unwrap_or_default()
                    },
                    move |dirs| {
                        populate_dir_list(&group_ui, &dir_rows_ui, dirs, &pool_ui);
                    },
                );
            });
        }
        row.add_suffix(&btn_del);

        group.add(&row);
        dir_rows.borrow_mut().push((dir.id, row));
    }
}

fn run_in_background<T, F, D>(rt: tokio::runtime::Handle, task: F, on_done: D)
where
    T: Send + 'static,
    F: Future<Output = T> + Send + 'static,
    D: FnOnce(T) + 'static,
{
    ui::run_in_background(rt, task, on_done);
}
