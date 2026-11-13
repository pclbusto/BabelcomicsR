use anyhow::Result;
use rustyline::{
    Context, Helper,
    completion::{Completer, Pair},
    error::ReadlineError,
    highlight::Highlighter,
    hint::Hinter,
    validate::Validator,
};
use sqlx::{Row, SqlitePool};

pub async fn run(pool: &SqlitePool) -> Result<()> {
    println!(
        "Modo interactivo de Babelcomics. Escribí 'help' para ver comandos, 'exit' para salir.\n"
    );

    let history_path = get_history_path();
    let mut config = crate::commands::config::ReplConfig::load();

    let mut rl = rustyline::Editor::<BabelHelper, rustyline::history::DefaultHistory>::new()?;
    rl.set_helper(Some(BabelHelper));

    if let Some(ref path) = history_path {
        let _ = rl.load_history(path);
    }

    loop {
        let readline = rl.readline("[babelcomics]# ");
        match readline {
            Ok(line) => {
                let line_trimmed = line.trim();
                if line_trimmed.is_empty() {
                    continue;
                }
                rl.add_history_entry(line_trimmed)?;

                let parts: Vec<&str> = line_trimmed.split_whitespace().collect();
                if parts.is_empty() {
                    continue;
                }

                match parts[0] {
                    "help" | "h" => print_help(),
                    "exit" | "quit" | "q" => {
                        println!("Saliendo...");
                        break;
                    }
                    "entities" => {
                        println!("\n📦 Entidades disponibles en Babelcomics:\n");
                        for (name, desc) in crate::commands::config::all_entities() {
                            println!("  {:<20} — {}", name, desc);
                        }
                        println!();
                    }
                    "open" => {
                        if parts.len() < 2 {
                            println!("Uso: open <id_comicbook>");
                            continue;
                        }
                        let id = parts[1].parse::<i64>().unwrap_or(0);
                        if id == 0 {
                            println!("ID inválido: {}", parts[1]);
                            continue;
                        }
                        let row = sqlx::query("SELECT path FROM comicbooks WHERE id_comicbook = ?")
                            .bind(id)
                            .fetch_optional(pool)
                            .await;
                        match row {
                            Ok(Some(r)) => {
                                let path: String = r.try_get("path").unwrap_or_default();
                                if path.is_empty() {
                                    println!("Comic {} no tiene path registrado.", id);
                                    continue;
                                }
                                let parent = std::path::Path::new(&path)
                                    .parent()
                                    .map(|p| p.to_string_lossy().to_string())
                                    .unwrap_or_else(|| path.clone());
                                #[cfg(target_os = "linux")]
                                let cmd = format!("xdg-open \"{}\"", parent);
                                #[cfg(target_os = "macos")]
                                let cmd = format!("open \"{}\"", parent);
                                #[cfg(not(any(target_os = "linux", target_os = "macos")))]
                                let cmd = format!("start \"{}\"", parent);
                                println!("Abriendo: {}", parent);
                                let _ =
                                    std::process::Command::new("sh").arg("-c").arg(&cmd).spawn();
                            }
                            Ok(None) => println!("No se encontró el comic con ID {}.", id),
                            Err(e) => eprintln!("Error: {}", e),
                        }
                    }
                    "columns" => {
                        if parts.len() < 2 {
                            println!("Uso: columns <comics|volumes|publishers>");
                            continue;
                        }
                        let entity = parts[1];
                        let available = crate::commands::config::available_columns(entity);
                        if available.is_empty() {
                            println!("Entidad desconocida: {}", entity);
                            continue;
                        }
                        let current = config.get_columns(entity);
                        println!("\nColumnas disponibles para '{}':", entity);
                        for (col, desc) in available {
                            let marker = if current.contains(&col.to_string()) {
                                "✓"
                            } else {
                                " "
                            };
                            println!("  [{}] {:<25} — {}", marker, col, desc);
                        }
                        println!("\nColumnas activas: {}", current.join(", "));
                        println!("\nPara cambiar: config columns <entity> col1 col2 col3");
                    }
                    "config" => {
                        if parts.len() < 2 {
                            println!("Uso: config columns <entity> <col1> [col2] ...");
                            println!("      config reset <entity>");
                            println!("      config show");
                            continue;
                        }
                        if parts[1] == "show" {
                            if config.columns.is_empty() {
                                println!(
                                    "Sin configuración personalizada. Usando valores por defecto."
                                );
                            } else {
                                for (entity, cols) in &config.columns {
                                    println!("  {}: {}", entity, cols.join(", "));
                                }
                            }
                            continue;
                        }
                        if parts.len() < 3 {
                            println!("Uso: config columns <entity> <col1> [col2] ...");
                            println!("      config reset <entity>");
                            println!("      config show");
                            continue;
                        }
                        match parts[1] {
                            "columns" => {
                                let entity = parts[2];
                                if parts.len() < 4 {
                                    println!("Uso: config columns <entity> <col1> [col2] ...");
                                    continue;
                                }
                                let cols: Vec<String> =
                                    parts[3..].iter().map(|s| s.to_string()).collect();
                                config.set_columns(entity, cols);
                                if let Err(e) = config.save() {
                                    eprintln!("Error guardando config: {}", e);
                                } else {
                                    println!("Configuración guardada para '{}'.", entity);
                                }
                            }
                            "reset" => {
                                let entity = parts[2];
                                config.columns.remove(entity);
                                if let Err(e) = config.save() {
                                    eprintln!("Error guardando config: {}", e);
                                } else {
                                    println!(
                                        "Configuración de '{}' restablecida a valores por defecto.",
                                        entity
                                    );
                                }
                            }
                            "show" => {
                                if config.columns.is_empty() {
                                    println!(
                                        "Sin configuración personalizada. Usando valores por defecto."
                                    );
                                } else {
                                    for (entity, cols) in &config.columns {
                                        println!("  {}: {}", entity, cols.join(", "));
                                    }
                                }
                            }
                            _ => println!("Comando config desconocido: {}", parts[1]),
                        }
                    }
                    "list" => {
                        if parts.len() < 2 {
                            println!(
                                "Uso: list <comics|comicbook_info|comicbook_detail|volumes|publishers> [opciones]"
                            );
                            continue;
                        }
                        let entity = parts[1];
                        let mut page = 1u32;
                        let mut per_page = 20u32;
                        let mut publisher: Option<String> = None;
                        let mut volume: Option<String> = None;
                        let mut title: Option<String> = None;
                        let mut path_filter: Option<String> = None;
                        let mut trash = false;
                        let mut unread = false;
                        let mut unprocessed = false;
                        let mut uncataloged = false;

                        let mut i = 2;
                        while i < parts.len() {
                            match parts[i] {
                                "-n" | "--page" => {
                                    if i + 1 < parts.len() {
                                        page = parts[i + 1].parse().unwrap_or(1);
                                        i += 1;
                                    }
                                }
                                "-p" | "--per-page" => {
                                    if i + 1 < parts.len() {
                                        per_page = parts[i + 1].parse().unwrap_or(20);
                                        i += 1;
                                    }
                                }
                                "-P" | "--publisher" => {
                                    if i + 1 < parts.len() {
                                        publisher = Some(parts[i + 1].to_string());
                                        i += 1;
                                    }
                                }
                                "-V" | "--volume" => {
                                    if i + 1 < parts.len() {
                                        volume = Some(parts[i + 1].to_string());
                                        i += 1;
                                    }
                                }
                                "-t" | "--title" => {
                                    if i + 1 < parts.len() {
                                        title = Some(parts[i + 1].to_string());
                                        i += 1;
                                    }
                                }
                                "-f" | "--path" => {
                                    if i + 1 < parts.len() {
                                        path_filter = Some(parts[i + 1].to_string());
                                        i += 1;
                                    }
                                }
                                "-T" | "--trash" => trash = true,
                                "-u" | "--unread" => unread = true,
                                "-U" | "--unprocessed" => unprocessed = true,
                                "-C" | "--uncataloged" => uncataloged = true,
                                _ => {}
                            }
                            i += 1;
                        }

                        let cols = config.get_columns(entity);

                        match entity {
                            "comics" | "c" => {
                                let filters = crate::ListFilters {
                                    publisher: publisher.clone(),
                                    volume: volume.clone(),
                                    title: title.clone(),
                                    path: path_filter.clone(),
                                    trash,
                                    unread,
                                    unprocessed,
                                    uncataloged,
                                };
                                if let Err(e) = crate::commands::list::list_comics(
                                    pool, page, per_page, &filters, &cols,
                                )
                                .await
                                {
                                    eprintln!("Error: {}", e);
                                }
                            }
                            "volumes" | "v" => {
                                if let Err(e) = crate::commands::list::list_volumes(
                                    pool, page, per_page, &publisher, &cols,
                                )
                                .await
                                {
                                    eprintln!("Error: {}", e);
                                }
                            }
                            "publishers" | "p" => {
                                if let Err(e) = crate::commands::list::list_publishers(
                                    pool, page, per_page, &cols,
                                )
                                .await
                                {
                                    eprintln!("Error: {}", e);
                                }
                            }
                            "comicbook_info" | "ci" => {
                                if let Err(e) = crate::commands::list::list_comicbook_info(
                                    pool, page, per_page, &cols,
                                )
                                .await
                                {
                                    eprintln!("Error: {}", e);
                                }
                            }
                            "comicbook_detail" | "cd" => {
                                if let Err(e) = crate::commands::list::list_comicbook_detail(
                                    pool, page, per_page, &cols,
                                )
                                .await
                                {
                                    eprintln!("Error: {}", e);
                                }
                            }
                            _ => println!(
                                "Entidad desconocida: {}. Usá 'comics', 'comicbook_info', 'comicbook_detail', 'volumes' o 'publishers'.",
                                entity
                            ),
                        }
                    }
                    "scan" => {
                        if parts.len() < 2 {
                            println!("Uso: scan <directorio>");
                            continue;
                        }
                        let dir = parts[1];
                        println!("Escaneando: {}", dir);
                        let dirs = vec![dir.to_string()];
                        match babelcomics_core::helpers::scan_service::run_scan(
                            pool,
                            &dirs,
                            babelcomics_core::helpers::thumbnail::CardSize::Medium,
                        )
                        .await
                        {
                            Ok(result) => println!(
                                "Escaneo finalizado. Encontrados: {}, Nuevos: {}, Portadas: {}, Errores: {}",
                                result.total_found,
                                result.new_inserted,
                                result.covers_generated,
                                result.errors.len()
                            ),
                            Err(e) => eprintln!("Error en escaneo: {}", e),
                        }
                    }
                    "db" => {
                        if parts.len() < 2 {
                            println!("Uso: db <info|migrate>");
                            continue;
                        }
                        match parts[1] {
                            "info" => {
                                let total_comics: i64 =
                                    sqlx::query_scalar("SELECT COUNT(*) FROM comicbooks")
                                        .fetch_one(pool)
                                        .await
                                        .unwrap_or(0);
                                let total_volumes: i64 =
                                    sqlx::query_scalar("SELECT COUNT(*) FROM volumens")
                                        .fetch_one(pool)
                                        .await
                                        .unwrap_or(0);
                                let total_publishers: i64 =
                                    sqlx::query_scalar("SELECT COUNT(*) FROM publishers")
                                        .fetch_one(pool)
                                        .await
                                        .unwrap_or(0);
                                let total_info: i64 =
                                    sqlx::query_scalar("SELECT COUNT(*) FROM comicbooks_info")
                                        .fetch_one(pool)
                                        .await
                                        .unwrap_or(0);
                                println!("📊 Estadísticas de la base de datos:");
                                println!("   Comics:      {}", total_comics);
                                println!("   Volúmenes:   {}", total_volumes);
                                println!("   Editoriales: {}", total_publishers);
                                println!("   Metadatos:   {}", total_info);
                            }
                            _ => println!("Comando db desconocido: {}", parts[1]),
                        }
                    }
                    "thumb" | "thumbnail" => {
                        println!("Generando thumbnails faltantes...");
                        match babelcomics_core::helpers::scan_service::generate_missing_thumbnails(
                            pool,
                            babelcomics_core::helpers::thumbnail::CardSize::Medium,
                        )
                        .await
                        {
                            Ok(result) => println!(
                                "Generación finalizada. Generados: {}, Errores: {}",
                                result.covers_generated,
                                result.errors.len()
                            ),
                            Err(e) => eprintln!("Error: {}", e),
                        }
                    }
                    _ => println!(
                        "Comando desconocido: {}. Escribí 'help' para ver la lista.",
                        parts[0]
                    ),
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("^D");
                break;
            }
            Err(err) => {
                eprintln!("Error: {:?}", err);
                break;
            }
        }
    }

    if let Some(ref path) = history_path {
        let _ = rl.save_history(path);
    }

    Ok(())
}

fn get_history_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let dir = std::path::PathBuf::from(format!("{}/.local/share/babelcomics", home));
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("cli_history"))
}

fn print_help() {
    println!(
        r#"
Comandos disponibles:
  list comics [opciones]        Listar cómics
  list comicbook_info [opts]    Listar metadatos ComicVine
  list comicbook_detail [opts]  Listar páginas de comics
  list volumes [opciones]       Listar volúmenes
  list publishers [opciones]    Listar editoriales
  entities                      Listar todas las entidades disponibles
  open <id>                     Abrir directorio del comic en el file manager
  columns <entity>              Ver columnas disponibles para una entidad
  config columns <e> c1 c2    Configurar columnas persistentes
  config reset <entity>         Restablecer columnas a valores por defecto
  config show                   Mostrar configuración actual
  scan <directorio>             Escanear directorio de cómics
  db info                       Mostrar estadísticas de la BD
  thumbnail                     Generar thumbnails faltantes
  help                          Mostrar esta ayuda
  exit                          Salir del modo interactivo

Opciones de list:
  -n, --page <N>             Página (default: 1)
  -p, --per-page <N>         Items por página (default: 20)
  -P, --publisher <TEXT>       Filtrar por editorial
  -V, --volume <TEXT>          Filtrar por volumen
  -t, --title <TEXT>           Filtrar por título
  -f, --path <TEXT>          Filtrar por texto en path del archivo
  -T, --trash                Solo papelera
  -u, --unread               Solo sin calificación
  -U, --unprocessed          Solo no procesados
  -C, --uncataloged          Solo sin metadatos ComicVine

Ejemplos:
  list comics -p 5 --publisher Marvel
  list comics --uncataloged --path "the avengers"
  columns comics
  config columns comics id_comicbook path titulo numero
  list volumes -p 10
  list publishers
"#
    );
}

// ─── Autocompletador ───────────────────────────────────────────────

#[derive(Default)]
struct BabelHelper;

impl Completer for BabelHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Pair>), ReadlineError> {
        let words: Vec<&str> = line[..pos].split_whitespace().collect();
        let current_word = if line.ends_with(' ') || words.is_empty() {
            ""
        } else {
            words.last().unwrap_or(&"")
        };

        let mut candidates: Vec<Pair> = Vec::new();

        if words.is_empty() || (words.len() == 1 && !line.ends_with(' ')) {
            let commands = [
                "list",
                "scan",
                "db",
                "thumbnail",
                "thumb",
                "open",
                "columns",
                "config",
                "help",
                "exit",
                "quit",
                "q",
            ];
            for cmd in commands {
                if cmd.starts_with(current_word) {
                    candidates.push(Pair {
                        display: cmd.to_string(),
                        replacement: cmd.to_string(),
                    });
                }
            }
        } else if words.len() >= 1 {
            let first = words[0];
            let second_word = if words.len() == 1 {
                ""
            } else if words.len() == 2 && !line.ends_with(' ') {
                words[1]
            } else {
                ""
            };

            match first {
                "list" => {
                    if words.len() == 1 || (words.len() == 2 && !line.ends_with(' ')) {
                        let subcmds = [
                            "comics",
                            "comicbook_info",
                            "comicbook_detail",
                            "volumes",
                            "publishers",
                            "c",
                            "ci",
                            "cd",
                            "v",
                            "p",
                        ];
                        for sub in subcmds {
                            if sub.starts_with(second_word) {
                                candidates.push(Pair {
                                    display: sub.to_string(),
                                    replacement: sub.to_string(),
                                });
                            }
                        }
                    } else {
                        let flags = [
                            "-n",
                            "--page",
                            "-p",
                            "--per-page",
                            "-P",
                            "--publisher",
                            "-V",
                            "--volume",
                            "-t",
                            "--title",
                            "-f",
                            "--path",
                            "-T",
                            "--trash",
                            "-u",
                            "--unread",
                            "-U",
                            "--unprocessed",
                            "-C",
                            "--uncataloged",
                        ];
                        for flag in flags {
                            if flag.starts_with(current_word) {
                                candidates.push(Pair {
                                    display: flag.to_string(),
                                    replacement: flag.to_string(),
                                });
                            }
                        }
                    }
                }
                "columns" | "config" => {
                    if words.len() == 1 || (words.len() == 2 && !line.ends_with(' ')) {
                        let subcmds = [
                            "comics",
                            "comicbook_info",
                            "comicbook_detail",
                            "volumes",
                            "publishers",
                            "c",
                            "ci",
                            "cd",
                            "v",
                            "p",
                        ];
                        for sub in subcmds {
                            if sub.starts_with(second_word) {
                                candidates.push(Pair {
                                    display: sub.to_string(),
                                    replacement: sub.to_string(),
                                });
                            }
                        }
                    }
                    if first == "config" && words.len() >= 2 {
                        let third = if words.len() == 2 && !line.ends_with(' ') {
                            ""
                        } else if words.len() == 3 && !line.ends_with(' ') {
                            words[2]
                        } else {
                            ""
                        };
                        let subcmds = ["columns", "reset", "show"];
                        for sub in subcmds {
                            if sub.starts_with(third) {
                                candidates.push(Pair {
                                    display: sub.to_string(),
                                    replacement: sub.to_string(),
                                });
                            }
                        }
                    }
                }
                "db" => {
                    if words.len() == 1 || (words.len() == 2 && !line.ends_with(' ')) {
                        let subcmds = ["info"];
                        for sub in subcmds {
                            if sub.starts_with(second_word) {
                                candidates.push(Pair {
                                    display: sub.to_string(),
                                    replacement: sub.to_string(),
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        let start = if current_word.is_empty() {
            pos
        } else {
            line[..pos].rfind(current_word).unwrap_or(pos)
        };

        Ok((start, candidates))
    }
}

impl Validator for BabelHelper {}
impl Hinter for BabelHelper {
    type Hint = String;
}
impl Highlighter for BabelHelper {}
impl Helper for BabelHelper {}
