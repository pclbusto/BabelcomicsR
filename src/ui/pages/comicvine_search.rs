use adw::prelude::*;
use gdk_pixbuf;
use gtk4::prelude::*;
use gtk4::{self as gtk, gdk};
use libadwaita as adw;
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use babelcomics_core::helpers::comicvine_client::ComicVineClient;
use babelcomics_core::helpers::download_manager::DownloadManager;
use babelcomics_core::helpers::publisher_import_service;
use babelcomics_core::repositories::SetupRepository;
use crate::ui::run_in_background;

// ── Tipos internos ────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq)]
enum SearchMode {
    Volumes,
    Publishers,
}

enum SearchResults {
    Volumes(BTreeMap<String, Vec<Value>>),
    Publishers(Vec<Value>),
    Empty,
}

// ── Punto de entrada ──────────────────────────────────────────────────────────

pub fn build(pool: SqlitePool) -> gtk::Widget {
    let main_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .build();

    // ── Header ────────────────────────────────────────────────────────────────
    let search_bar_bin = adw::Bin::new();
    let search_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    // Fila superior: Dropdown | Título centrado | Botón acción
    let header_top = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();

    let context_combo = gtk::ComboBoxText::new();
    context_combo.append(Some("volumes"), "Volúmenes");
    context_combo.append(Some("publishers"), "Editoriales");
    context_combo.set_active_id(Some("volumes"));
    header_top.append(&context_combo);

    let title = gtk::Label::builder()
        .label("Buscar en ComicVine")
        .halign(gtk::Align::Center)
        .css_classes(["title-2"])
        .hexpand(true)
        .build();
    header_top.append(&title);

    let btn_action = gtk::Button::builder()
        .label("Descargar Seleccionados")
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    header_top.append(&btn_action);

    search_box.append(&header_top);

    // Fila de búsqueda
    let search_entry_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();

    let search_entry = gtk::Entry::builder()
        .placeholder_text("Nombre del volumen (ej: Batman, Spider-Man...)")
        .hexpand(true)
        .build();
    search_entry_box.append(&search_entry);

    let btn_search = gtk::Button::builder()
        .label("Buscar")
        .css_classes(["suggested-action"])
        .build();
    search_entry_box.append(&btn_search);

    let spinner = gtk::Spinner::builder().valign(gtk::Align::Center).build();
    search_entry_box.append(&spinner);

    search_box.append(&search_entry_box);

    // Etiqueta de estado (importación)
    let status_label = gtk::Label::builder()
        .halign(gtk::Align::Center)
        .css_classes(["dim-label", "caption"])
        .visible(false)
        .build();
    search_box.append(&status_label);

    // Filtros (solo visibles en modo Volúmenes)
    let filters_revealer = gtk::Revealer::builder()
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .reveal_child(true)
        .build();

    let filters_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .halign(gtk::Align::Center)
        .build();

    let year_box = gtk::Box::builder().spacing(8).build();
    year_box.append(&gtk::Label::new(Some("Año:")));
    let spin_year_from = gtk::SpinButton::with_range(1930.0, 2030.0, 1.0);
    spin_year_from.set_value(1950.0);
    let spin_year_to = gtk::SpinButton::with_range(1930.0, 2030.0, 1.0);
    spin_year_to.set_value(2025.0);
    year_box.append(&spin_year_from);
    year_box.append(&gtk::Label::new(Some("—")));
    year_box.append(&spin_year_to);
    filters_box.append(&year_box);

    let pub_box = gtk::Box::builder().spacing(8).build();
    pub_box.append(&gtk::Label::new(Some("Editorial:")));
    let combo_pub = gtk::ComboBoxText::builder().build();
    combo_pub.append(None, "Todas");
    combo_pub.set_active(Some(0));
    pub_box.append(&combo_pub);
    filters_box.append(&pub_box);

    filters_revealer.set_child(Some(&filters_box));
    search_box.append(&filters_revealer);

    search_bar_bin.set_child(Some(&search_box));
    main_box.append(&search_bar_bin);

    // ── Área de Resultados ────────────────────────────────────────────────────
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .build();

    let results_flowbox = gtk::FlowBox::builder()
        .valign(gtk::Align::Start)
        .max_children_per_line(4)
        .min_children_per_line(1)
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .column_spacing(15)
        .row_spacing(15)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    scrolled.set_child(Some(&results_flowbox));
    main_box.append(&scrolled);

    // ── Estado compartido ─────────────────────────────────────────────────────
    let selected_volumes: Arc<Mutex<HashMap<String, Value>>> = Arc::new(Mutex::new(HashMap::new()));
    let selected_publishers: Arc<Mutex<HashMap<String, Value>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // ── Cambio de modo (Dropdown) ─────────────────────────────────────────────
    {
        let filters_rev = filters_revealer.clone();
        let entry = search_entry.clone();
        let btn = btn_action.clone();
        let flow = results_flowbox.clone();
        let sel_v = selected_volumes.clone();
        let sel_p = selected_publishers.clone();
        let status = status_label.clone();

        context_combo.connect_changed(move |combo| {
            let is_volumes = combo.active_id().map(|id| id == "volumes").unwrap_or(true);
            filters_rev.set_reveal_child(is_volumes);

            if is_volumes {
                entry.set_placeholder_text(Some("Nombre del volumen (ej: Batman, Spider-Man...)"));
                btn.set_label("Descargar Seleccionados");
            } else {
                entry.set_placeholder_text(Some(
                    "Nombre de la editorial (ej: Marvel, DC Comics...)",
                ));
                btn.set_label("Importar Seleccionados");
            }

            // Limpiar selecciones y resultados al cambiar modo
            sel_v.lock().unwrap().clear();
            sel_p.lock().unwrap().clear();
            btn.set_sensitive(false);
            status.set_visible(false);
            while let Some(child) = flow.first_child() {
                flow.remove(&child);
            }
        });
    }

    // ── Búsqueda ──────────────────────────────────────────────────────────────
    {
        let pool_s = pool.clone();
        let flow = results_flowbox.clone();
        let spin = spinner.clone();
        let btn_s = btn_search.clone();
        let btn_a = btn_action.clone();
        let sel_v = selected_volumes.clone();
        let sel_p = selected_publishers.clone();
        let combo = context_combo.clone();
        let status = status_label.clone();

        btn_search.connect_clicked(move |_| {
            let query = search_entry.text().to_string();
            if query.is_empty() { return; }

            let mode = match combo.active_id().as_deref() {
                Some("publishers") => SearchMode::Publishers,
                _ => SearchMode::Volumes,
            };

            spin.start();
            btn_s.set_sensitive(false);
            btn_a.set_sensitive(false);
            sel_v.lock().unwrap().clear();
            sel_p.lock().unwrap().clear();
            status.set_visible(false);

            while let Some(child) = flow.first_child() {
                flow.remove(&child);
            }

            let p = pool_s.clone();
            let flow2 = flow.clone();
            let spin2 = spin.clone();
            let btn_s2 = btn_s.clone();
            let btn_a2 = btn_a.clone();
            let sel_v2 = sel_v.clone();
            let sel_p2 = sel_p.clone();

            run_in_background(
                tokio::runtime::Handle::current(),
                async move {
                    let setup = SetupRepository::new(&p).get().await.unwrap_or_default();
                    let api_key = match setup.api_key_encrypted.filter(|k| !k.is_empty()) {
                        Some(k) => k,
                        None => return SearchResults::Empty,
                    };
                    let client = match ComicVineClient::new(api_key, setup.api_url) {
                        Ok(c) => c,
                        Err(_) => return SearchResults::Empty,
                    };

                    match mode {
                        SearchMode::Publishers => {
                            let results = client.get_publishers(50, 0, Some(&query)).await;
                            SearchResults::Publishers(results)
                        }
                        SearchMode::Volumes => {
                            let volumes = client.get_volumes(Some(&query), None).await;
                            let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
                            for vol in volumes {
                                let name = vol["name"].as_str().unwrap_or("").to_lowercase();
                                let pub_name = vol["publisher"]["name"]
                                    .as_str()
                                    .unwrap_or("unknown")
                                    .to_lowercase();
                                let key = format!("{}|{}", name, pub_name);
                                groups.entry(key).or_default().push(vol);
                            }
                            SearchResults::Volumes(groups)
                        }
                    }
                },
                move |results| {
                    let has_content = match results {
                        SearchResults::Empty => {
                            show_empty_msg(&flow2, "Sin resultados. Verifica que tengas la API key configurada en Preferencias.");
                            false
                        }
                        SearchResults::Volumes(groups) => {
                            if groups.is_empty() {
                                show_empty_msg(&flow2, "Sin resultados para esa búsqueda.");
                                false
                            } else {
                                for (_key, vols) in groups {
                                    let card = build_group_card(vols, sel_v2.clone(), btn_a2.clone());
                                    flow2.append(&card);
                                }
                                true
                            }
                        }
                        SearchResults::Publishers(publishers) => {
                            if publishers.is_empty() {
                                show_empty_msg(&flow2, "No se encontraron editoriales con ese nombre.");
                                false
                            } else {
                                for pub_data in publishers {
                                    let card = build_publisher_card(pub_data, sel_p2.clone(), btn_a2.clone());
                                    flow2.append(&card);
                                }
                                true
                            }
                        }
                    };
                    let _ = has_content;
                    spin2.stop();
                    btn_s2.set_sensitive(true);
                },
            );
        });
    }

    // ── Acción del botón principal (Descargar / Importar) ─────────────────────
    {
        let p_dl = pool.clone();
        let sel_v = selected_volumes.clone();
        let sel_p = selected_publishers.clone();
        let combo = context_combo.clone();
        let btn_a = btn_action.clone();
        let status = status_label.clone();

        btn_action.connect_clicked(move |btn| {
            let is_publishers = combo
                .active_id()
                .map(|id| id == "publishers")
                .unwrap_or(false);

            if is_publishers {
                // ── Importar editoriales ──────────────────────────────────────
                let pubs = sel_p.lock().unwrap().clone();
                if pubs.is_empty() {
                    return;
                }

                btn.set_sensitive(false);
                btn.set_label("Importando...");
                status.set_visible(true);
                status.set_label("Preparando importación...");

                let p = p_dl.clone();
                let btn_ref = btn_a.clone();
                let status_ref = status.clone();
                let pub_list: Vec<Value> = pubs.into_values().collect();

                run_in_background(
                    tokio::runtime::Handle::current(),
                    async move {
                        let setup = SetupRepository::new(&p).get().await.unwrap_or_default();
                        let api_key = match setup.api_key_encrypted.filter(|k| !k.is_empty()) {
                            Some(k) => k,
                            None => return Err(anyhow::anyhow!("API key no configurada.")),
                        };
                        let client = match ComicVineClient::new(api_key, setup.api_url) {
                            Ok(c) => c,
                            Err(e) => return Err(e),
                        };

                        let mut total_inserted = 0usize;
                        let mut total_skipped = 0usize;
                        let mut names: Vec<String> = Vec::new();

                        for pub_data in &pub_list {
                            let cv_id = match pub_data["id"].as_i64() {
                                Some(id) => id,
                                None => continue,
                            };
                            match publisher_import_service::import_publisher_from_cv(
                                &p, &client, cv_id,
                            )
                            .await
                            {
                                Ok(report) => {
                                    total_inserted += report.volumes_inserted;
                                    total_skipped += report.volumes_skipped;
                                    names.push(report.publisher_name);
                                }
                                Err(e) => {
                                    tracing::error!("Error importando editorial {}: {}", cv_id, e);
                                }
                            }
                        }

                        Ok((names, total_inserted, total_skipped))
                    },
                    move |result| {
                        match result {
                            Ok((names, inserted, skipped)) => {
                                let label = format!(
                                    "✓ {} importada(s). {} volúmenes nuevos, {} ya existían.",
                                    names.join(", "),
                                    inserted,
                                    skipped,
                                );
                                status_ref.set_label(&label);
                            }
                            Err(e) => {
                                status_ref.set_label(&format!("Error: {}", e));
                            }
                        }
                        btn_ref.set_label("Importar Seleccionados");
                        btn_ref.set_sensitive(true);
                    },
                );
            } else {
                // ── Descargar volúmenes ───────────────────────────────────────
                let vols = sel_v.lock().unwrap().clone();
                if vols.is_empty() {
                    return;
                }

                let dm = DownloadManager::get_instance(p_dl.clone());
                for (_, vol_data) in vols {
                    dm.add_download(vol_data, true);
                }
            }
        });
    }

    main_box.upcast()
}

// ── Helpers de UI ─────────────────────────────────────────────────────────────

fn show_empty_msg(flow: &gtk::FlowBox, text: &str) {
    let lbl = gtk::Label::builder()
        .label(text)
        .wrap(true)
        .halign(gtk::Align::Center)
        .css_classes(["dim-label"])
        .build();
    flow.append(&lbl);
}

fn build_group_card(
    volumes: Vec<Value>,
    selected_volumes: Arc<Mutex<HashMap<String, Value>>>,
    btn_action: gtk::Button,
) -> gtk::Widget {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .css_classes(["card"])
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    if volumes.len() > 1 {
        let carousel = adw::Carousel::builder().spacing(12).build();
        for vol in volumes {
            carousel.append(&build_volume_widget(
                &vol,
                selected_volumes.clone(),
                btn_action.clone(),
            ));
        }
        let dots = adw::CarouselIndicatorDots::builder()
            .carousel(&carousel)
            .margin_bottom(4)
            .build();
        card.append(&carousel);
        card.append(&dots);
    } else if let Some(vol) = volumes.first() {
        card.append(&build_volume_widget(vol, selected_volumes, btn_action));
    }

    card.upcast()
}

fn build_volume_widget(
    vol: &Value,
    selected_volumes: Arc<Mutex<HashMap<String, Value>>>,
    btn_action: gtk::Button,
) -> gtk::Widget {
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();

    let vol_clone = vol.clone();
    let cv_id = vol["id"].as_u64().unwrap_or(0).to_string();

    let img_url = vol["image"]["medium_url"].as_str().unwrap_or("");
    let picture = gtk::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .height_request(200)
        .build();

    if !img_url.is_empty() {
        let picture_clone = picture.clone();
        let url = img_url.to_string();
        run_in_background(
            tokio::runtime::Handle::current(),
            async move { reqwest::get(url).await.ok()?.bytes().await.ok() },
            move |bytes| {
                if let Some(b) = bytes {
                    let loader = gdk_pixbuf::PixbufLoader::new();
                    if loader.write(&b).is_ok() && loader.close().is_ok() {
                        if let Some(pix) = loader.pixbuf() {
                            picture_clone.set_paintable(Some(&gdk::Texture::for_pixbuf(&pix)));
                        }
                    }
                }
            },
        );
    }
    container.append(&picture);

    let lbl_title = gtk::Label::builder()
        .label(vol["name"].as_str().unwrap_or("Sin título"))
        .css_classes(["title-4"])
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .halign(gtk::Align::Start)
        .build();
    container.append(&lbl_title);

    let year = vol["start_year"].as_str().unwrap_or("N/A");
    let pub_name = vol["publisher"]["name"]
        .as_str()
        .unwrap_or("Editorial desconocida");
    let lbl_info = gtk::Label::builder()
        .label(&format!("{} ({})", pub_name, year))
        .css_classes(["caption", "dim-label"])
        .halign(gtk::Align::Start)
        .build();
    container.append(&lbl_info);

    let check = gtk::CheckButton::builder().label("Seleccionar").build();
    let sel_ref = selected_volumes.clone();
    let btn_ref = btn_action.clone();
    let id_key = cv_id.clone();

    check.connect_toggled(move |cb| {
        let mut sel = sel_ref.lock().unwrap();
        if cb.is_active() {
            sel.insert(id_key.clone(), vol_clone.clone());
        } else {
            sel.remove(&id_key);
        }
        btn_ref.set_sensitive(!sel.is_empty());
    });
    container.append(&check);

    container.upcast()
}

fn build_publisher_card(
    pub_data: Value,
    selected_publishers: Arc<Mutex<HashMap<String, Value>>>,
    btn_action: gtk::Button,
) -> gtk::Widget {
    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .css_classes(["card"])
        .margin_top(4)
        .margin_bottom(4)
        .margin_start(4)
        .margin_end(4)
        .build();

    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();

    // Logo
    let img_url = pub_data["image"]["medium_url"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let picture = gtk::Picture::builder()
        .can_shrink(true)
        .content_fit(gtk::ContentFit::Contain)
        .height_request(200)
        .build();

    if !img_url.is_empty() {
        let picture_clone = picture.clone();
        let url = img_url.clone();
        run_in_background(
            tokio::runtime::Handle::current(),
            async move { reqwest::get(url).await.ok()?.bytes().await.ok() },
            move |bytes| {
                if let Some(b) = bytes {
                    let loader = gdk_pixbuf::PixbufLoader::new();
                    if loader.write(&b).is_ok() && loader.close().is_ok() {
                        if let Some(pix) = loader.pixbuf() {
                            picture_clone.set_paintable(Some(&gdk::Texture::for_pixbuf(&pix)));
                        }
                    }
                }
            },
        );
    }
    container.append(&picture);

    // Nombre
    let name = pub_data["name"]
        .as_str()
        .unwrap_or("Sin nombre")
        .to_string();
    let lbl_name = gtk::Label::builder()
        .label(&name)
        .css_classes(["title-4"])
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .halign(gtk::Align::Start)
        .build();
    container.append(&lbl_name);

    // Subtítulo: deck o ciudad de la editorial
    let subtitle = pub_data["deck"]
        .as_str()
        .filter(|s| !s.is_empty())
        .or_else(|| pub_data["location_city"].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("");

    if !subtitle.is_empty() {
        let lbl_sub = gtk::Label::builder()
            .label(subtitle)
            .css_classes(["caption", "dim-label"])
            .halign(gtk::Align::Start)
            .wrap(true)
            .max_width_chars(30)
            .build();
        container.append(&lbl_sub);
    }

    // Cantidad de volúmenes conocidos (si la API lo devuelve)
    if let Some(count) = pub_data["count_of_issues"].as_u64().or_else(|| {
        pub_data["volume_credits"]
            .as_array()
            .map(|a| a.len() as u64)
    }) {
        if count > 0 {
            let lbl_count = gtk::Label::builder()
                .label(&format!("{} números", count))
                .css_classes(["caption", "dim-label"])
                .halign(gtk::Align::Start)
                .build();
            container.append(&lbl_count);
        }
    }

    // Checkbox de selección
    let check = gtk::CheckButton::builder().label("Seleccionar").build();
    let cv_id = pub_data["id"].as_u64().unwrap_or(0).to_string();
    let sel_ref = selected_publishers.clone();
    let btn_ref = btn_action.clone();
    let id_key = cv_id.clone();

    check.connect_toggled(move |cb| {
        let mut sel = sel_ref.lock().unwrap();
        if cb.is_active() {
            sel.insert(id_key.clone(), pub_data.clone());
        } else {
            sel.remove(&id_key);
        }
        btn_ref.set_sensitive(!sel.is_empty());
    });
    container.append(&check);

    card.append(&container);
    card.upcast()
}
