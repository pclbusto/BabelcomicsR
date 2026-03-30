use std::sync::{Arc, Mutex};
use std::collections::{BTreeMap, HashMap};
use gtk4::prelude::*;
use gtk4::{self as gtk, gio, glib, gdk};
use libadwaita as adw;
use adw::prelude::*;
use sqlx::SqlitePool;
use serde_json::Value;
use gdk_pixbuf;

use crate::repositories::{SetupRepository};
use crate::helpers::comicvine_client::ComicVineClient;
use crate::helpers::download_manager::DownloadManager;
use crate::ui::run_in_background;

pub fn build(pool: SqlitePool) -> gtk::Widget {
    let main_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .build();

    // --- Header de Búsqueda ---
    let search_bar_bin = adw::Bin::new();
    let search_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let header_top = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .build();

    let title = gtk::Label::builder()
        .label("Buscar en ComicVine")
        .halign(gtk::Align::Start)
        .css_classes(["title-2"])
        .hexpand(true)
        .build();
    header_top.append(&title);

    let btn_download = gtk::Button::builder()
        .label("Descargar Seleccionados")
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    header_top.append(&btn_download);

    search_box.append(&header_top);

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

    let spinner = gtk::Spinner::builder()
        .valign(gtk::Align::Center)
        .build();
    search_entry_box.append(&spinner);

    search_box.append(&search_entry_box);

    // --- Filtros ---
    let filters_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(16)
        .halign(gtk::Align::Center)
        .build();

    // Año
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

    // Editorial (Combo dinámico)
    let pub_box = gtk::Box::builder().spacing(8).build();
    pub_box.append(&gtk::Label::new(Some("Editorial:")));
    let combo_pub = gtk::ComboBoxText::builder().build();
    combo_pub.append(None, "Todas");
    combo_pub.set_active(Some(0));
    pub_box.append(&combo_pub);
    filters_box.append(&pub_box);

    search_box.append(&filters_box);
    search_bar_bin.set_child(Some(&search_box));
    main_box.append(&search_bar_bin);

    // --- Área de Resultados ---
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

    // --- Lógica de Búsqueda ---
    let pool_search = pool.clone();
    let flowbox_ref = results_flowbox.clone();
    let spinner_ref = spinner.clone();
    let btn_ref = btn_search.clone();
    let btn_dl_ref = btn_download.clone();

    // Estado compartido para rastrear volúmenes seleccionados
    let selected_volumes: Arc<Mutex<HashMap<String, Value>>> = Arc::new(Mutex::new(HashMap::new()));

    let selected_volumes_for_search = selected_volumes.clone();
    btn_search.connect_clicked(move |_| {
        let query = search_entry.text().to_string();
        if query.is_empty() { return; }

        spinner_ref.start();
        btn_ref.set_sensitive(false);
        btn_dl_ref.set_sensitive(false);
        selected_volumes_for_search.lock().unwrap().clear();
        
        // Limpiar resultados
        while let Some(child) = flowbox_ref.first_child() {
            flowbox_ref.remove(&child);
        }

        let p = pool_search.clone();
        let flow = flowbox_ref.clone();
        let spin = spinner_ref.clone();
        let btn = btn_ref.clone();
        let btn_dl = btn_dl_ref.clone();
        let sel_vol = selected_volumes_for_search.clone();

        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                let setup = SetupRepository::new(&p).get().await.unwrap_or_default();
                let client = ComicVineClient::new(
                    setup.api_key_encrypted.unwrap_or_default(),
                    setup.api_url
                ).unwrap();

                // 1. Buscar volúmenes
                let volumes = client.get_volumes(Some(&query), None).await;
                
                // 2. Agrupar por nombre + editorial
                let mut groups: BTreeMap<String, Vec<Value>> = BTreeMap::new();
                for vol in volumes {
                    let name = vol["name"].as_str().unwrap_or("").to_lowercase();
                    let pub_name = vol["publisher"]["name"].as_str().unwrap_or("unknown").to_lowercase();
                    let key = format!("{}|{}", name, pub_name);
                    groups.entry(key).or_default().push(vol);
                }
                groups
            },
            move |groups| {
                for (_key, volumes) in groups {
                    let card = build_group_card(volumes, sel_vol.clone(), btn_dl.clone());
                    flow.append(&card);
                }
                spin.stop();
                btn.set_sensitive(true);
            }
        );
    });

    // --- Lógica de Descarga ---
    let p_dl = pool.clone();
    let selected_volumes_for_download = selected_volumes.clone();
    btn_download.connect_clicked(move |_| {
        let volumes_to_dl = selected_volumes_for_download.lock().unwrap().clone();
        if volumes_to_dl.is_empty() { return; }

        let pool_task = p_dl.clone();
        run_in_background(
            tokio::runtime::Handle::current(),
            async move {
                let setup = SetupRepository::new(&pool_task).get().await.unwrap_or_default();
                let client = ComicVineClient::new(
                    setup.api_key_encrypted.unwrap_or_default(),
                    setup.api_url
                ).unwrap();
                let dm = DownloadManager::get_instance(pool_task);

                for (_, vol_data) in volumes_to_dl {
                    dm.add_download(vol_data, client.clone(), true);
                }
            },
            |_| {
                tracing::info!("Descargas iniciadas");
            }
        );
    });

    main_box.upcast()
}

fn build_group_card(
    volumes: Vec<Value>, 
    selected_volumes: Arc<Mutex<HashMap<String, Value>>>,
    btn_download: gtk::Button
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
        let carousel = adw::Carousel::builder()
            .spacing(12)
            .build();
        
        for vol in volumes {
            carousel.append(&build_volume_widget(&vol, selected_volumes.clone(), btn_download.clone()));
        }

        let dots = adw::CarouselIndicatorDots::builder()
            .carousel(&carousel)
            .margin_bottom(4)
            .build();
        
        card.append(&carousel);
        card.append(&dots);
    } else if let Some(vol) = volumes.first() {
        card.append(&build_volume_widget(vol, selected_volumes, btn_download));
    }

    card.upcast()
}

fn build_volume_widget(
    vol: &Value, 
    selected_volumes: Arc<Mutex<HashMap<String, Value>>>,
    btn_download: gtk::Button
) -> gtk::Widget {
    let container = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(4)
        .build();
    
    let vol_clone = vol.clone();
    let cv_id = vol["id"].as_u64().unwrap_or(0).to_string();

    // Imagen
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
            async move {
                reqwest::get(url).await.ok()?.bytes().await.ok()
            },
            move |bytes| {
                if let Some(b) = bytes {
                    let loader = gdk_pixbuf::PixbufLoader::new();
                    if loader.write(&b).is_ok() && loader.close().is_ok() {
                        if let Some(pix) = loader.pixbuf() {
                            picture_clone.set_paintable(Some(&gdk::Texture::for_pixbuf(&pix)));
                        }
                    }
                }
            }
        );
    }
    container.append(&picture);

    // Título
    let title = vol["name"].as_str().unwrap_or("Sin título");
    let lbl_title = gtk::Label::builder()
        .label(title)
        .css_classes(["title-4"])
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .halign(gtk::Align::Start)
        .build();
    container.append(&lbl_title);

    let year = vol["start_year"].as_str().unwrap_or("N/A");
    let pub_name = vol["publisher"]["name"].as_str().unwrap_or("Editorial desconocida");
    let lbl_info = gtk::Label::builder()
        .label(&format!("{} ({})", pub_name, year))
        .css_classes(["caption", "dim-label"])
        .halign(gtk::Align::Start)
        .build();
    container.append(&lbl_info);

    // Checkbox de selección
    let check = gtk::CheckButton::builder()
        .label("Seleccionar")
        .build();
    
    let sel_vol_ref = selected_volumes.clone();
    let btn_dl_ref = btn_download.clone();
    let id_key = cv_id.clone();
    
    check.connect_toggled(move |cb| {
        let mut selected = sel_vol_ref.lock().unwrap();
        if cb.is_active() {
            selected.insert(id_key.clone(), vol_clone.clone());
        } else {
            selected.remove(&id_key);
        }
        btn_dl_ref.set_sensitive(!selected.is_empty());
    });

    container.append(&check);
    container.upcast()
}
