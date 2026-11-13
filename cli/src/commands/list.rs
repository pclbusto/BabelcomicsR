use anyhow::Result;
use sqlx::{Row, SqlitePool};

pub async fn list_comics(
    pool: &SqlitePool,
    page: u32,
    per_page: u32,
    filters: &crate::ListFilters,
    columns: &[String],
) -> Result<()> {
    let offset = (page.saturating_sub(1) as i64) * (per_page as i64);
    let limit = per_page as i64;

    let mut conditions: Vec<String> = Vec::new();
    if filters.trash {
        conditions.push("cb.en_papelera = 1".to_string());
    }
    if filters.unread {
        conditions.push("(cbi.calificacion IS NULL OR cbi.calificacion = 0)".to_string());
    }
    if filters.unprocessed {
        conditions.push("cb.procesado = 0".to_string());
    }
    if filters.publisher.is_some() {
        conditions.push("p.nombre LIKE ?".to_string());
    }
    if filters.volume.is_some() {
        conditions.push("v.nombre LIKE ?".to_string());
    }
    if filters.title.is_some() {
        conditions.push("cbi.titulo LIKE ?".to_string());
    }
    if filters.path.is_some() {
        conditions.push("cb.path LIKE ?".to_string());
    }
    if filters.uncataloged {
        conditions.push("cb.id_comicbook_info IS NULL".to_string());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let select_sql = format!(
        r#"
        SELECT
            cb.id_comicbook,
            cb.path,
            cb.en_papelera,
            cb.calidad,
            cb.error_ultimo_escaneo,
            cb.procesado,
            cbi.titulo,
            cbi.numero,
            cbi.calificacion,
            v.nombre as nombre_volume,
            p.nombre as nombre_publisher
        FROM comicbooks cb
        LEFT JOIN comicbooks_info cbi ON cb.id_comicbook_info = cbi.id_comicbook_info
        LEFT JOIN volumens v ON cbi.id_volume = v.id_volume
        LEFT JOIN publishers p ON v.id_publisher = p.id_publisher
        {}
        ORDER BY cb.id_comicbook
        LIMIT ? OFFSET ?
        "#,
        where_clause
    );

    let mut query = sqlx::query(&select_sql);
    if let Some(ref p) = filters.publisher {
        query = query.bind(format!("%{}%", p));
    }
    if let Some(ref v) = filters.volume {
        query = query.bind(format!("%{}%", v));
    }
    if let Some(ref t) = filters.title {
        query = query.bind(format!("%{}%", t));
    }
    if let Some(ref f) = filters.path {
        query = query.bind(format!("%{}%", f));
    }
    query = query.bind(limit).bind(offset);
    let rows = query.fetch_all(pool).await?;

    let count_sql = format!(
        "SELECT COUNT(*) FROM comicbooks cb LEFT JOIN comicbooks_info cbi ON cb.id_comicbook_info = cbi.id_comicbook_info LEFT JOIN volumens v ON cbi.id_volume = v.id_volume LEFT JOIN publishers p ON v.id_publisher = p.id_publisher{}",
        where_clause
    );
    let mut count_query = sqlx::query_scalar(&count_sql);
    if let Some(ref p) = filters.publisher {
        count_query = count_query.bind(format!("%{}%", p));
    }
    if let Some(ref v) = filters.volume {
        count_query = count_query.bind(format!("%{}%", v));
    }
    if let Some(ref t) = filters.title {
        count_query = count_query.bind(format!("%{}%", t));
    }
    if let Some(ref f) = filters.path {
        count_query = count_query.bind(format!("%{}%", f));
    }
    let total: i64 = count_query.fetch_one(pool).await?;

    if rows.is_empty() {
        println!(
            "No hay cómics para mostrar en la página {} con los filtros aplicados.",
            page
        );
        return Ok(());
    }

    // ── Configuración de columnas ──────────────────────────────────
    let show_all = columns.is_empty() || columns.contains(&"*".to_string());
    let show_id = show_all || columns.contains(&"id_comicbook".to_string());
    let show_path = show_all || columns.contains(&"path".to_string());
    let show_titulo = show_all || columns.contains(&"titulo".to_string());
    let show_numero = show_all || columns.contains(&"numero".to_string());
    let show_vol = show_all || columns.contains(&"nombre_volume".to_string());
    let show_pub = show_all || columns.contains(&"nombre_publisher".to_string());
    let show_cal = show_all || columns.contains(&"calificacion".to_string());
    let show_pap = show_all || columns.contains(&"en_papelera".to_string());
    let show_calidad = show_all || columns.contains(&"calidad".to_string());
    let show_procesado = show_all || columns.contains(&"procesado".to_string());
    let show_error = show_all || columns.contains(&"error_ultimo_escaneo".to_string());

    let get_str = |r: &sqlx::sqlite::SqliteRow, col: &str| -> Option<String> {
        r.try_get::<String, _>(col).ok()
    };

    // ── Calcular anchos dinámicos ──────────────────────────────────
    let mut w_id = 5usize;
    let mut w_path = 30usize;
    let mut w_titulo = 20usize;
    let mut w_numero = 6usize;
    let mut w_vol = 20usize;
    let mut w_pub = 15usize;
    let mut w_cal = 3usize;
    let mut w_pap = 3usize;
    let mut w_calidad = 8usize;
    let mut w_procesado = 10usize;
    let mut w_error = 25usize;

    for r in &rows {
        if show_path {
            if let Some(v) = get_str(r, "path") {
                w_path = w_path.max(v.chars().count().min(60));
            }
        }
        if show_titulo {
            if let Some(v) = get_str(r, "titulo") {
                w_titulo = w_titulo.max(v.chars().count());
            }
        }
        if show_vol {
            if let Some(v) = get_str(r, "nombre_volume") {
                w_vol = w_vol.max(v.chars().count());
            }
        }
        if show_pub {
            if let Some(v) = get_str(r, "nombre_publisher") {
                w_pub = w_pub.max(v.chars().count());
            }
        }
        if show_error {
            if let Some(v) = get_str(r, "error_ultimo_escaneo") {
                w_error = w_error.max(v.chars().count().min(40));
            }
        }
    }

    // ── Cabecera ───────────────────────────────────────────────────
    let total_pages = if total == 0 {
        1
    } else {
        ((total as f64) / (per_page as f64)).ceil() as u32
    };
    println!(
        "\n📚 Página {}/{} — Mostrando {} de {} cómics\n",
        page,
        total_pages,
        rows.len(),
        total
    );

    let mut header_parts: Vec<String> = Vec::new();
    let mut separator_parts: Vec<usize> = Vec::new();

    if show_id {
        header_parts.push(format!(" {:>width$}", "ID", width = w_id));
        separator_parts.push(w_id);
    }
    if show_path {
        header_parts.push(format!(" {:<width$}", "Path", width = w_path));
        separator_parts.push(w_path);
    }
    if show_titulo {
        header_parts.push(format!(" {:<width$}", "Título", width = w_titulo));
        separator_parts.push(w_titulo);
    }
    if show_numero {
        header_parts.push(format!(" {:>width$}", "Num", width = w_numero));
        separator_parts.push(w_numero);
    }
    if show_vol {
        header_parts.push(format!(" {:<width$}", "Volumen", width = w_vol));
        separator_parts.push(w_vol);
    }
    if show_pub {
        header_parts.push(format!(" {:<width$}", "Editorial", width = w_pub));
        separator_parts.push(w_pub);
    }
    if show_cal {
        header_parts.push(format!(" {:>width$}", "Cal", width = w_cal));
        separator_parts.push(w_cal);
    }
    if show_pap {
        header_parts.push(format!(" {:>width$}", "Pap", width = w_pap));
        separator_parts.push(w_pap);
    }
    if show_calidad {
        header_parts.push(format!(" {:>width$}", "Calidad", width = w_calidad));
        separator_parts.push(w_calidad);
    }
    if show_procesado {
        header_parts.push(format!(" {:>width$}", "Procesado", width = w_procesado));
        separator_parts.push(w_procesado);
    }
    if show_error {
        header_parts.push(format!(" {:<width$}", "Error", width = w_error));
        separator_parts.push(w_error);
    }

    let total_width: usize =
        separator_parts.iter().sum::<usize>() + (separator_parts.len().saturating_sub(1) * 3);
    println!("{}", header_parts.join(" │ "));
    println!("{:-<width$}", "", width = total_width);

    // ── Filas ──────────────────────────────────────────────────────
    for r in rows {
        let mut parts: Vec<String> = Vec::new();

        if show_id {
            let id: i64 = r.try_get("id_comicbook").unwrap_or(0);
            parts.push(format!(" {:>width$}", id, width = w_id));
        }
        if show_path {
            let path = get_str(&r, "path").unwrap_or_else(|| "—".to_string());
            let truncated = &path[..path.chars().count().min(w_path)];
            let linked = if path != "—" {
                crate::commands::config::osc8_link(truncated, &path)
            } else {
                truncated.to_string()
            };
            parts.push(format!(" {:<width$}", linked, width = w_path));
        }
        if show_titulo {
            let titulo = get_str(&r, "titulo").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:<width$}",
                &titulo[..titulo.chars().count().min(w_titulo)],
                width = w_titulo
            ));
        }
        if show_numero {
            let numero = get_str(&r, "numero").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:>width$}",
                &numero[..numero.chars().count().min(w_numero)],
                width = w_numero
            ));
        }
        if show_vol {
            let vol = get_str(&r, "nombre_volume").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:<width$}",
                &vol[..vol.chars().count().min(w_vol)],
                width = w_vol
            ));
        }
        if show_pub {
            let pub_ = get_str(&r, "nombre_publisher").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:<width$}",
                &pub_[..pub_.chars().count().min(w_pub)],
                width = w_pub
            ));
        }
        if show_cal {
            let cal = r
                .try_get::<f64, _>("calificacion")
                .map(|c| format!("{:.1}", c))
                .unwrap_or_else(|_| "—".to_string());
            parts.push(format!(" {:>width$}", cal, width = w_cal));
        }
        if show_pap {
            let en_papelera: bool = r.try_get("en_papelera").unwrap_or(false);
            parts.push(format!(
                " {:>width$}",
                if en_papelera { "Sí" } else { "No" },
                width = w_pap
            ));
        }
        if show_calidad {
            let calidad = get_str(&r, "calidad").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:>width$}",
                &calidad[..calidad.chars().count().min(w_calidad)],
                width = w_calidad
            ));
        }
        if show_procesado {
            let procesado: bool = r.try_get("procesado").unwrap_or(false);
            parts.push(format!(
                " {:>width$}",
                if procesado { "Sí" } else { "No" },
                width = w_procesado
            ));
        }
        if show_error {
            let error = get_str(&r, "error_ultimo_escaneo").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:<width$}",
                &error[..error.chars().count().min(w_error)],
                width = w_error
            ));
        }

        println!("{}", parts.join(" │ "));
    }

    println!();
    Ok(())
}

pub async fn list_volumes(
    pool: &SqlitePool,
    page: u32,
    per_page: u32,
    publisher_filter: &Option<String>,
    columns: &[String],
) -> Result<()> {
    let offset = (page.saturating_sub(1) as i64) * (per_page as i64);
    let limit = per_page as i64;

    let mut conditions: Vec<String> = Vec::new();
    if publisher_filter.is_some() {
        conditions.push("p.nombre LIKE ?".to_string());
    }

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let select_sql = format!(
        r#"
        SELECT
            v.id_volume,
            v.nombre,
            v.deck,
            v.anio_inicio,
            v.cantidad_numeros,
            p.nombre as nombre_publisher
        FROM volumens v
        LEFT JOIN publishers p ON v.id_publisher = p.id_publisher
        {}
        ORDER BY v.nombre
        LIMIT ? OFFSET ?
        "#,
        where_clause
    );

    let mut query = sqlx::query(&select_sql);
    if let Some(p) = publisher_filter {
        query = query.bind(format!("%{}%", p));
    }
    query = query.bind(limit).bind(offset);
    let rows = query.fetch_all(pool).await?;

    let count_sql = format!(
        "SELECT COUNT(*) FROM volumens v LEFT JOIN publishers p ON v.id_publisher = p.id_publisher{}",
        where_clause
    );
    let mut count_query = sqlx::query_scalar(&count_sql);
    if let Some(p) = publisher_filter {
        count_query = count_query.bind(format!("%{}%", p));
    }
    let total: i64 = count_query.fetch_one(pool).await?;

    if rows.is_empty() {
        println!("No hay volúmenes para mostrar en la página {}.", page);
        return Ok(());
    }

    let show_all = columns.is_empty() || columns.contains(&"*".to_string());
    let show_id = show_all || columns.contains(&"id_volume".to_string());
    let show_nombre = show_all || columns.contains(&"nombre".to_string());
    let show_deck = show_all || columns.contains(&"deck".to_string());
    let show_anio = show_all || columns.contains(&"anio_inicio".to_string());
    let show_nums = show_all || columns.contains(&"cantidad_numeros".to_string());
    let show_pub = show_all || columns.contains(&"nombre_publisher".to_string());

    let get_str = |r: &sqlx::sqlite::SqliteRow, col: &str| -> Option<String> {
        r.try_get::<String, _>(col).ok()
    };

    let mut w_id = 5usize;
    let mut w_nombre = 30usize;
    let mut w_deck = 25usize;
    let mut w_anio = 4usize;
    let mut w_nums = 5usize;
    let mut w_pub = 20usize;

    for r in &rows {
        if show_nombre {
            if let Some(v) = get_str(r, "nombre") {
                w_nombre = w_nombre.max(v.chars().count());
            }
        }
        if show_deck {
            if let Some(v) = get_str(r, "deck") {
                w_deck = w_deck.max(v.chars().count().min(40));
            }
        }
        if show_pub {
            if let Some(v) = get_str(r, "nombre_publisher") {
                w_pub = w_pub.max(v.chars().count());
            }
        }
    }

    let total_pages = if total == 0 {
        1
    } else {
        ((total as f64) / (per_page as f64)).ceil() as u32
    };
    println!(
        "\n📚 Página {}/{} — Mostrando {} de {} volúmenes\n",
        page,
        total_pages,
        rows.len(),
        total
    );

    let mut header_parts: Vec<String> = Vec::new();
    let mut sep: Vec<usize> = Vec::new();
    if show_id {
        header_parts.push(format!(" {:>width$}", "ID", width = w_id));
        sep.push(w_id);
    }
    if show_nombre {
        header_parts.push(format!(" {:<width$}", "Nombre", width = w_nombre));
        sep.push(w_nombre);
    }
    if show_deck {
        header_parts.push(format!(" {:<width$}", "Deck", width = w_deck));
        sep.push(w_deck);
    }
    if show_anio {
        header_parts.push(format!(" {:>width$}", "Año", width = w_anio));
        sep.push(w_anio);
    }
    if show_nums {
        header_parts.push(format!(" {:>width$}", "#Num", width = w_nums));
        sep.push(w_nums);
    }
    if show_pub {
        header_parts.push(format!(" {:<width$}", "Editorial", width = w_pub));
        sep.push(w_pub);
    }

    let total_width: usize = sep.iter().sum::<usize>() + (sep.len().saturating_sub(1) * 3);
    println!("{}", header_parts.join(" │ "));
    println!("{:-<width$}", "", width = total_width);

    for r in rows {
        let mut parts: Vec<String> = Vec::new();
        if show_id {
            parts.push(format!(
                " {:>width$}",
                r.try_get::<i64, _>("id_volume").unwrap_or(0),
                width = w_id
            ));
        }
        if show_nombre {
            let v = get_str(&r, "nombre").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:<width$}",
                &v[..v.chars().count().min(w_nombre)],
                width = w_nombre
            ));
        }
        if show_deck {
            let v = get_str(&r, "deck").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:<width$}",
                &v[..v.chars().count().min(w_deck)],
                width = w_deck
            ));
        }
        if show_anio {
            parts.push(format!(
                " {:>width$}",
                r.try_get::<i64, _>("anio_inicio").unwrap_or(0),
                width = w_anio
            ));
        }
        if show_nums {
            parts.push(format!(
                " {:>width$}",
                r.try_get::<i64, _>("cantidad_numeros").unwrap_or(0),
                width = w_nums
            ));
        }
        if show_pub {
            let v = get_str(&r, "nombre_publisher").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:<width$}",
                &v[..v.chars().count().min(w_pub)],
                width = w_pub
            ));
        }
        println!("{}", parts.join(" │ "));
    }

    println!();
    Ok(())
}

pub async fn list_publishers(
    pool: &SqlitePool,
    page: u32,
    per_page: u32,
    columns: &[String],
) -> Result<()> {
    let offset = (page.saturating_sub(1) as i64) * (per_page as i64);
    let limit = per_page as i64;

    let rows = sqlx::query(
        r#"
        SELECT
            id_publisher,
            nombre,
            descripcion,
            id_comicvine
        FROM publishers
        ORDER BY nombre
        LIMIT ? OFFSET ?
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM publishers")
        .fetch_one(pool)
        .await?;

    if rows.is_empty() {
        println!("No hay editoriales para mostrar en la página {}.", page);
        return Ok(());
    }

    let show_all = columns.is_empty() || columns.contains(&"*".to_string());
    let show_id = show_all || columns.contains(&"id_publisher".to_string());
    let show_nombre = show_all || columns.contains(&"nombre".to_string());
    let show_desc = show_all || columns.contains(&"descripcion".to_string());
    let show_cv = show_all || columns.contains(&"id_comicvine".to_string());

    let get_str = |r: &sqlx::sqlite::SqliteRow, col: &str| -> Option<String> {
        r.try_get::<String, _>(col).ok()
    };

    let mut w_id = 5usize;
    let mut w_nombre = 25usize;
    let mut w_desc = 60usize;
    let mut w_cv = 10usize;

    for r in &rows {
        if show_nombre {
            if let Some(v) = get_str(r, "nombre") {
                w_nombre = w_nombre.max(v.chars().count());
            }
        }
        if show_desc {
            if let Some(v) = get_str(r, "descripcion") {
                w_desc = w_desc.max(v.chars().count().min(60));
            }
        }
    }

    let total_pages = if total == 0 {
        1
    } else {
        ((total as f64) / (per_page as f64)).ceil() as u32
    };
    println!(
        "\n📚 Página {}/{} — Mostrando {} de {} editoriales\n",
        page,
        total_pages,
        rows.len(),
        total
    );

    let mut header_parts: Vec<String> = Vec::new();
    let mut sep: Vec<usize> = Vec::new();
    if show_id {
        header_parts.push(format!(" {:>width$}", "ID", width = w_id));
        sep.push(w_id);
    }
    if show_nombre {
        header_parts.push(format!(" {:<width$}", "Nombre", width = w_nombre));
        sep.push(w_nombre);
    }
    if show_desc {
        header_parts.push(format!(" {:<width$}", "Descripción", width = w_desc));
        sep.push(w_desc);
    }
    if show_cv {
        header_parts.push(format!(" {:>width$}", "ComicVine ID", width = w_cv));
        sep.push(w_cv);
    }

    let total_width: usize = sep.iter().sum::<usize>() + (sep.len().saturating_sub(1) * 3);
    println!("{}", header_parts.join(" │ "));
    println!("{:-<width$}", "", width = total_width);

    for r in rows {
        let mut parts: Vec<String> = Vec::new();
        if show_id {
            parts.push(format!(
                " {:>width$}",
                r.try_get::<i64, _>("id_publisher").unwrap_or(0),
                width = w_id
            ));
        }
        if show_nombre {
            let v = get_str(&r, "nombre").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:<width$}",
                &v[..v.chars().count().min(w_nombre)],
                width = w_nombre
            ));
        }
        if show_desc {
            let v_raw = get_str(&r, "descripcion").unwrap_or_else(|| "—".to_string());
            let v = if v_raw.len() > 60 {
                format!("{}...", &v_raw[..57])
            } else {
                v_raw
            };
            parts.push(format!(
                " {:<width$}",
                &v[..v.chars().count().min(w_desc)],
                width = w_desc
            ));
        }
        if show_cv {
            let cv: Option<i64> = r.try_get("id_comicvine").ok();
            parts.push(format!(
                " {:>width$}",
                cv.map(|v| v.to_string()).unwrap_or_else(|| "—".to_string()),
                width = w_cv
            ));
        }
        println!("{}", parts.join(" │ "));
    }

    println!();
    Ok(())
}

pub async fn list_comicbook_info(
    pool: &SqlitePool,
    page: u32,
    per_page: u32,
    columns: &[String],
) -> Result<()> {
    let offset = (page.saturating_sub(1) as i64) * (per_page as i64);
    let limit = per_page as i64;

    let rows = sqlx::query(
        r#"
        SELECT
            ci.id_comicbook_info,
            ci.titulo,
            ci.id_volume,
            ci.numero,
            ci.resumen,
            ci.calificacion,
            ci.id_comicvine,
            ci.url_api_detalle,
            ci.fue_actualizado_api,
            v.nombre as nombre_volume
        FROM comicbooks_info ci
        LEFT JOIN volumens v ON ci.id_volume = v.id_volume
        ORDER BY ci.id_comicbook_info
        LIMIT ? OFFSET ?
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM comicbooks_info")
        .fetch_one(pool)
        .await?;

    if rows.is_empty() {
        println!("No hay metadatos para mostrar en la página {}.", page);
        return Ok(());
    }

    let show_all = columns.is_empty() || columns.contains(&"*".to_string());
    let show_id = show_all || columns.contains(&"id_comicbook_info".to_string());
    let show_titulo = show_all || columns.contains(&"titulo".to_string());
    let show_id_vol = show_all || columns.contains(&"id_volume".to_string());
    let show_numero = show_all || columns.contains(&"numero".to_string());
    let show_resumen = show_all || columns.contains(&"resumen".to_string());
    let show_cal = show_all || columns.contains(&"calificacion".to_string());
    let show_cv = show_all || columns.contains(&"id_comicvine".to_string());
    let show_url = show_all || columns.contains(&"url_api_detalle".to_string());
    let show_updated = show_all || columns.contains(&"fue_actualizado_api".to_string());
    let show_vol = show_all || columns.contains(&"nombre_volume".to_string());

    let get_str = |r: &sqlx::sqlite::SqliteRow, col: &str| -> Option<String> {
        r.try_get::<String, _>(col).ok()
    };

    let mut w_id = 5usize;
    let mut w_titulo = 25usize;
    let mut w_numero = 6usize;
    let mut w_vol = 20usize;
    let mut w_cal = 3usize;
    let mut w_cv = 10usize;

    for r in &rows {
        if show_titulo {
            if let Some(v) = get_str(r, "titulo") {
                w_titulo = w_titulo.max(v.chars().count());
            }
        }
        if show_vol {
            if let Some(v) = get_str(r, "nombre_volume") {
                w_vol = w_vol.max(v.chars().count());
            }
        }
    }

    let total_pages = if total == 0 {
        1
    } else {
        ((total as f64) / (per_page as f64)).ceil() as u32
    };
    println!(
        "\n📚 Página {}/{} — Mostrando {} de {} metadatos\n",
        page,
        total_pages,
        rows.len(),
        total
    );

    let mut header_parts: Vec<String> = Vec::new();
    let mut sep: Vec<usize> = Vec::new();
    if show_id {
        header_parts.push(format!(" {:>width$}", "ID", width = w_id));
        sep.push(w_id);
    }
    if show_titulo {
        header_parts.push(format!(" {:<width$}", "Título", width = w_titulo));
        sep.push(w_titulo);
    }
    if show_numero {
        header_parts.push(format!(" {:>width$}", "Num", width = w_numero));
        sep.push(w_numero);
    }
    if show_vol {
        header_parts.push(format!(" {:<width$}", "Volumen", width = w_vol));
        sep.push(w_vol);
    }
    if show_cal {
        header_parts.push(format!(" {:>width$}", "Cal", width = w_cal));
        sep.push(w_cal);
    }
    if show_cv {
        header_parts.push(format!(" {:>width$}", "CV ID", width = w_cv));
        sep.push(w_cv);
    }
    if show_updated {
        header_parts.push(format!(" {:>width$}", "Act", width = 3));
        sep.push(3);
    }

    let total_width: usize = sep.iter().sum::<usize>() + (sep.len().saturating_sub(1) * 3);
    println!("{}", header_parts.join(" │ "));
    println!("{:-<width$}", "", width = total_width);

    for r in rows {
        let mut parts: Vec<String> = Vec::new();
        if show_id {
            parts.push(format!(
                " {:>width$}",
                r.try_get::<i64, _>("id_comicbook_info").unwrap_or(0),
                width = w_id
            ));
        }
        if show_titulo {
            let v = get_str(&r, "titulo").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:<width$}",
                &v[..v.chars().count().min(w_titulo)],
                width = w_titulo
            ));
        }
        if show_numero {
            let v = get_str(&r, "numero").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:>width$}",
                &v[..v.chars().count().min(w_numero)],
                width = w_numero
            ));
        }
        if show_vol {
            let v = get_str(&r, "nombre_volume").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:<width$}",
                &v[..v.chars().count().min(w_vol)],
                width = w_vol
            ));
        }
        if show_cal {
            let cal = r
                .try_get::<f64, _>("calificacion")
                .map(|c| format!("{:.1}", c))
                .unwrap_or_else(|_| "—".to_string());
            parts.push(format!(" {:>width$}", cal, width = w_cal));
        }
        if show_cv {
            let cv: Option<i64> = r.try_get("id_comicvine").ok();
            parts.push(format!(
                " {:>width$}",
                cv.map(|v| v.to_string()).unwrap_or_else(|| "—".to_string()),
                width = w_cv
            ));
        }
        if show_updated {
            let upd: bool = r.try_get("fue_actualizado_api").unwrap_or(false);
            parts.push(format!(
                " {:>width$}",
                if upd { "Sí" } else { "No" },
                width = 3
            ));
        }
        println!("{}", parts.join(" │ "));
    }

    println!();
    Ok(())
}

pub async fn list_comicbook_detail(
    pool: &SqlitePool,
    page: u32,
    per_page: u32,
    columns: &[String],
) -> Result<()> {
    let offset = (page.saturating_sub(1) as i64) * (per_page as i64);
    let limit = per_page as i64;

    let rows = sqlx::query(
        r#"
        SELECT
            id_detail,
            comicbook_id,
            indicePagina as indice_pagina,
            ordenPagina as orden_pagina,
            tipoPagina as tipo_pagina,
            nombre_pagina
        FROM comicbook_detail
        ORDER BY id_detail
        LIMIT ? OFFSET ?
        "#,
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM comicbook_detail")
        .fetch_one(pool)
        .await?;

    if rows.is_empty() {
        println!(
            "No hay detalles de páginas para mostrar en la página {}.",
            page
        );
        return Ok(());
    }

    let show_all = columns.is_empty() || columns.contains(&"*".to_string());
    let show_id = show_all || columns.contains(&"id_detail".to_string());
    let show_cb_id = show_all || columns.contains(&"comicbook_id".to_string());
    let show_indice = show_all || columns.contains(&"indice_pagina".to_string());
    let show_orden = show_all || columns.contains(&"orden_pagina".to_string());
    let show_tipo = show_all || columns.contains(&"tipo_pagina".to_string());
    let show_nombre = show_all || columns.contains(&"nombre_pagina".to_string());

    let get_str = |r: &sqlx::sqlite::SqliteRow, col: &str| -> Option<String> {
        r.try_get::<String, _>(col).ok()
    };

    let mut w_id = 5usize;
    let mut w_cb_id = 5usize;
    let mut w_indice = 6usize;
    let mut w_orden = 6usize;
    let mut w_tipo = 4usize;
    let mut w_nombre = 20usize;

    for r in &rows {
        if show_nombre {
            if let Some(v) = get_str(r, "nombre_pagina") {
                w_nombre = w_nombre.max(v.chars().count());
            }
        }
    }

    let total_pages = if total == 0 {
        1
    } else {
        ((total as f64) / (per_page as f64)).ceil() as u32
    };
    println!(
        "\n📚 Página {}/{} — Mostrando {} de {} detalles de páginas\n",
        page,
        total_pages,
        rows.len(),
        total
    );

    let mut header_parts: Vec<String> = Vec::new();
    let mut sep: Vec<usize> = Vec::new();
    if show_id {
        header_parts.push(format!(" {:>width$}", "ID", width = w_id));
        sep.push(w_id);
    }
    if show_cb_id {
        header_parts.push(format!(" {:>width$}", "CB", width = w_cb_id));
        sep.push(w_cb_id);
    }
    if show_indice {
        header_parts.push(format!(" {:>width$}", "Idx", width = w_indice));
        sep.push(w_indice);
    }
    if show_orden {
        header_parts.push(format!(" {:>width$}", "Ord", width = w_orden));
        sep.push(w_orden);
    }
    if show_tipo {
        header_parts.push(format!(" {:>width$}", "Tipo", width = w_tipo));
        sep.push(w_tipo);
    }
    if show_nombre {
        header_parts.push(format!(" {:<width$}", "Nombre", width = w_nombre));
        sep.push(w_nombre);
    }

    let total_width: usize = sep.iter().sum::<usize>() + (sep.len().saturating_sub(1) * 3);
    println!("{}", header_parts.join(" │ "));
    println!("{:-<width$}", "", width = total_width);

    for r in rows {
        let mut parts: Vec<String> = Vec::new();
        if show_id {
            parts.push(format!(
                " {:>width$}",
                r.try_get::<i64, _>("id_detail").unwrap_or(0),
                width = w_id
            ));
        }
        if show_cb_id {
            parts.push(format!(
                " {:>width$}",
                r.try_get::<i64, _>("comicbook_id").unwrap_or(0),
                width = w_cb_id
            ));
        }
        if show_indice {
            parts.push(format!(
                " {:>width$}",
                r.try_get::<i64, _>("indice_pagina").unwrap_or(0),
                width = w_indice
            ));
        }
        if show_orden {
            parts.push(format!(
                " {:>width$}",
                r.try_get::<i64, _>("orden_pagina").unwrap_or(0),
                width = w_orden
            ));
        }
        if show_tipo {
            parts.push(format!(
                " {:>width$}",
                r.try_get::<i64, _>("tipo_pagina").unwrap_or(0),
                width = w_tipo
            ));
        }
        if show_nombre {
            let v = get_str(&r, "nombre_pagina").unwrap_or_else(|| "—".to_string());
            parts.push(format!(
                " {:<width$}",
                &v[..v.chars().count().min(w_nombre)],
                width = w_nombre
            ));
        }
        println!("{}", parts.join(" │ "));
    }

    println!();
    Ok(())
}
