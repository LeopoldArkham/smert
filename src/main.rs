mod config;
mod daily;
mod db;
mod error;
mod import;
mod resources;

use config::Config;
use daily::Kind;
use resources::artwork::{self, Artwork};
use resources::definition::{self, Definition};
use resources::quote::{self, Quote};
use resources::scripture::{self, Scripture};

use askama::Template;
use axum::{
    Form, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use sqlx::SqlitePool;
use tower_http::services::ServeDir;

#[derive(Clone)]
struct AppState {
    pool: SqlitePool,
}

/// Any failure inside a handler: logged, and reported to the browser as a 500.
struct AppError(error::Error);

impl<E: Into<error::Error>> From<E> for AppError {
    fn from(e: E) -> Self {
        Self(e.into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        eprintln!("error: {}", self.0);
        (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response()
    }
}

type HandlerResult<T> = std::result::Result<T, AppError>;

#[derive(Template)]
#[template(path = "admin.html")]
struct AdminTemplate {
    quote_count:          usize,
    quotes:               Vec<Quote>,
    authors:              Vec<String>,
    works:                Vec<String>,
    quotes_json:          String,
    artwork_count:        usize,
    artworks:             Vec<Artwork>,
    artists:              Vec<String>,
    definition_count:     usize,
    definitions_en:       Vec<Definition>,
    definitions_fr:       Vec<Definition>,
    scripture_count:      usize,
    scriptures:           Vec<Scripture>,
    error:                Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct AdminQuery {
    error: Option<String>,
}

#[derive(Template)]
#[template(path = "wotd.html")]
struct WotdTemplate {
    word_en: Option<Definition>,
    word_fr: Option<Definition>,
}

#[derive(Template)]
#[template(path = "home.html")]
struct HomeTemplate {
    quote:     Quote,
    artwork:   Option<Artwork>,
    word_en:   Option<Definition>,
    word_fr:   Option<Definition>,
    scripture: Option<Scripture>,
}

#[derive(serde::Deserialize)]
struct DeleteForm {
    id: i64,
}

#[derive(serde::Deserialize)]
struct NewQuoteForm {
    text:     String,
    author:   String,
    work:     String,
    date:     String,
    language: String,
}

#[derive(serde::Deserialize)]
struct NewArtworkForm {
    url:    String,
    artist: String,
    work:   String,
    date:   String,
}

#[derive(serde::Deserialize)]
struct ResolveQuery {
    url: String,
}

#[derive(serde::Deserialize)]
struct NewDefinitionForm {
    word_fr: String,
    word_en: String,
}

#[derive(serde::Deserialize)]
struct NewScriptureForm {
    book:        String,
    chapter:     u16,
    verse_start: u16,
    verse_end:   Option<u16>,
    text:        String,
}

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    let pool = db::connect(&config).await.unwrap_or_else(|e| {
        eprintln!("failed to open database {}: {e}", config.database_path().display());
        std::process::exit(1);
    });

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => {}
        Some("import") => {
            let dir = args.get(1).map(String::as_str).unwrap_or(".");
            if let Err(e) = import::run(&pool, std::path::Path::new(dir)).await {
                eprintln!("import failed: {e}");
                std::process::exit(1);
            }
            return;
        }
        Some(other) => {
            eprintln!("unknown command {other:?}; usage: smert [import [DIR]]");
            std::process::exit(2);
        }
    }

    let state = AppState { pool };

    // Press "r" + Enter in the console to re-roll every daily pick on demand.
    let stdin_pool = state.pool.clone();
    let runtime = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            match stdin.read_line(&mut line) {
                Ok(0) => break, // EOF — stdin closed
                Ok(_) => {
                    if line.trim().eq_ignore_ascii_case("r") {
                        match runtime.block_on(reroll_all(&stdin_pool)) {
                            Ok(())  => println!("↻ Rerolled today's picks."),
                            Err(e)  => eprintln!("reroll failed: {e}"),
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });
    println!("Server listening on http://{}  (press \"r\" + Enter to reshuffle)", config.bind);

    let app = Router::new()
        .route("/",                          get(home))
        .route("/qotd",                      get(qotd))
        .route("/artwork",                   get(artwork_handler))
        .route("/wotd",                      get(wotd))
        .route("/admin",                     get(admin_get))
        .route("/admin/quotes",              post(admin_quotes_post))
        .route("/admin/quotes/delete",       post(admin_quotes_delete))
        .route("/admin/artworks",            post(admin_artworks_post))
        .route("/admin/artworks/resolve",    get(admin_artworks_resolve))
        .route("/admin/artworks/delete",     post(admin_artworks_delete))
        .route("/admin/definitions",         post(admin_definitions_post))
        .route("/admin/definitions/delete",  post(admin_definitions_delete))
        .route("/admin/scriptures",          post(admin_scriptures_post))
        .route("/admin/scriptures/delete",   post(admin_scriptures_delete))
        .nest_service("/static",             ServeDir::new("static"))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind).await.unwrap_or_else(|e| {
        eprintln!("failed to bind {}: {e}", config.bind);
        std::process::exit(1);
    });
    let _ = axum::serve(listener, app).await;
}

async fn reroll_all(pool: &SqlitePool) -> error::Result<()> {
    for kind in Kind::ALL {
        daily::reroll(pool, kind).await?;
    }
    Ok(())
}

// ── Display handlers ──────────────────────────────────────────────────────────

async fn qotd(State(state): State<AppState>) -> HandlerResult<(StatusCode, Html<String>)> {
    Ok(match daily::todays::<Quote>(&state.pool, Kind::Quote).await? {
        Some(q) => (StatusCode::OK, Html(quote::render_quote(&q)?)),
        None    => (StatusCode::NOT_FOUND, Html("<p>No quote yet.</p>".into())),
    })
}

async fn artwork_handler(State(state): State<AppState>) -> HandlerResult<(StatusCode, Html<String>)> {
    Ok(match daily::todays::<Artwork>(&state.pool, Kind::Artwork).await? {
        Some(a) => (StatusCode::OK, Html(artwork::render_artwork(&a)?)),
        None    => (StatusCode::NOT_FOUND, Html("<p>No artwork yet.</p>".into())),
    })
}

async fn wotd(State(state): State<AppState>) -> HandlerResult<Html<String>> {
    let word_en = daily::todays(&state.pool, Kind::WordEn).await?;
    let word_fr = daily::todays(&state.pool, Kind::WordFr).await?;
    Ok(Html(WotdTemplate { word_en, word_fr }.render()?))
}

async fn home(State(state): State<AppState>) -> HandlerResult<(StatusCode, Html<String>)> {
    let Some(quote) = daily::todays(&state.pool, Kind::Quote).await? else {
        return Ok((StatusCode::NOT_FOUND, Html("<p>No quote yet.</p>".into())));
    };
    let artwork   = daily::todays(&state.pool, Kind::Artwork).await?;
    let word_en   = daily::todays(&state.pool, Kind::WordEn).await?;
    let word_fr   = daily::todays(&state.pool, Kind::WordFr).await?;
    let scripture = daily::todays(&state.pool, Kind::Scripture).await?;
    let tmpl = HomeTemplate { quote, artwork, word_en, word_fr, scripture };
    Ok((StatusCode::OK, Html(tmpl.render()?)))
}

// ── Admin handlers ────────────────────────────────────────────────────────────

fn err_redirect(tab: &str, e: impl std::fmt::Display) -> Redirect {
    Redirect::to(&format!(
        "/admin?error={}#{}",
        urlencoding::encode(&e.to_string()),
        tab,
    ))
}

async fn admin_get(
    State(state): State<AppState>,
    Query(query): Query<AdminQuery>,
) -> HandlerResult<Html<String>> {
    let quotes         = quote::all(&state.pool).await?;
    let artworks       = artwork::all(&state.pool).await?;
    let definitions_en = definition::all(&state.pool, "english").await?;
    let definitions_fr = definition::all(&state.pool, "french").await?;
    let scriptures     = scripture::all(&state.pool).await?;
    let authors        = quote::unique_authors(&quotes);
    let works          = quote::unique_works(&quotes);
    let artists        = artwork::unique_artists(&artworks);
    let quotes_json    = serde_json::to_string(&quotes).unwrap_or_default();
    let definition_count     = definitions_en.len() + definitions_fr.len();
    let quote_count          = quotes.len();
    let artwork_count        = artworks.len();
    let scripture_count      = scriptures.len();

    let tmpl = AdminTemplate {
        quote_count,
        quotes,
        authors,
        works,
        quotes_json,
        artwork_count,
        artworks,
        artists,
        definition_count,
        definitions_en,
        definitions_fr,
        scripture_count,
        scriptures,
        error: query.error,
    };
    Ok(Html(tmpl.render()?))
}

async fn admin_quotes_post(State(state): State<AppState>, Form(form): Form<NewQuoteForm>) -> Redirect {
    let work = (!form.work.trim().is_empty()).then(|| form.work.trim().to_string());
    let date = (!form.date.trim().is_empty()).then(|| form.date.trim().to_string());
    match quote::add(&state.pool, &Quote {
        id:       0,
        text:     form.text.trim().to_string(),
        author:   form.author.trim().to_string(),
        work,
        date,
        language: form.language,
    }).await {
        Ok(())  => Redirect::to("/admin"),
        Err(e)  => err_redirect("quotes", e),
    }
}

async fn admin_quotes_delete(State(state): State<AppState>, Form(form): Form<DeleteForm>) -> HandlerResult<Redirect> {
    quote::delete(&state.pool, form.id).await?;
    Ok(Redirect::to("/admin"))
}

async fn admin_artworks_post(State(state): State<AppState>, Form(form): Form<NewArtworkForm>) -> Redirect {
    match artwork::download_and_add(
        &state.pool,
        form.url.trim(),
        form.artist.trim(),
        form.work.trim(),
        form.date.trim(),
    ).await {
        Ok(())  => Redirect::to("/admin#artworks"),
        Err(e)  => err_redirect("artworks", e),
    }
}

async fn admin_artworks_resolve(
    Query(q): Query<ResolveQuery>,
) -> Result<axum::Json<artwork::ResolvedArtwork>, (StatusCode, String)> {
    match artwork::resolve_from_wiki(q.url.trim()).await {
        Ok(resolved) => Ok(axum::Json(resolved)),
        Err(e)       => Err((StatusCode::BAD_REQUEST, e.to_string())),
    }
}

async fn admin_artworks_delete(State(state): State<AppState>, Form(form): Form<DeleteForm>) -> HandlerResult<Redirect> {
    artwork::delete(&state.pool, form.id).await?;
    Ok(Redirect::to("/admin#artworks"))
}

async fn admin_definitions_post(State(state): State<AppState>, Form(form): Form<NewDefinitionForm>) -> Redirect {
    let fr = form.word_fr.trim().to_string();
    let en = form.word_en.trim().to_string();
    let mut errors: Vec<String> = Vec::new();
    let fr_fut = async {
        if !fr.is_empty() {
            if let Err(e) = definition::fetch_and_add(&state.pool, &fr, "french").await {
                Some(format!("fr: {e}"))
            } else { None }
        } else { None }
    };
    let en_fut = async {
        if !en.is_empty() {
            if let Err(e) = definition::fetch_and_add(&state.pool, &en, "english").await {
                Some(format!("en: {e}"))
            } else { None }
        } else { None }
    };
    let (fr_err, en_err) = tokio::join!(fr_fut, en_fut);
    let fr_added = !fr.is_empty() && fr_err.is_none();
    let en_added = !en.is_empty() && en_err.is_none();
    errors.extend(fr_err);
    errors.extend(en_err);
    if errors.is_empty() {
        let mut langs: Vec<&str> = Vec::new();
        if fr_added { langs.push("french"); }
        if en_added { langs.push("english"); }
        if langs.is_empty() {
            Redirect::to("/admin#definitions")
        } else {
            Redirect::to(&format!("/admin?added={}#definitions", langs.join(",")))
        }
    } else {
        err_redirect("definitions", errors.join("; "))
    }
}

async fn admin_definitions_delete(State(state): State<AppState>, Form(form): Form<DeleteForm>) -> HandlerResult<Redirect> {
    definition::delete(&state.pool, form.id).await?;
    Ok(Redirect::to("/admin#definitions"))
}

async fn admin_scriptures_post(State(state): State<AppState>, Form(form): Form<NewScriptureForm>) -> Redirect {
    let verse_end = form.verse_end.filter(|&v| v > 0 && v != form.verse_start);
    match scripture::add(&state.pool, &Scripture {
        id:          0,
        book:        form.book.trim().to_string(),
        chapter:     form.chapter,
        verse_start: form.verse_start,
        verse_end,
        text:        form.text.trim().to_string(),
    }).await {
        Ok(())  => Redirect::to("/admin#scriptures"),
        Err(e)  => err_redirect("scriptures", e),
    }
}

async fn admin_scriptures_delete(State(state): State<AppState>, Form(form): Form<DeleteForm>) -> HandlerResult<Redirect> {
    scripture::delete(&state.pool, form.id).await?;
    Ok(Redirect::to("/admin#scriptures"))
}
