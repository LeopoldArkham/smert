mod config;
mod daily;
mod db;
mod error;
mod resources;

use config::Config;
use daily::Kind;
use resources::artwork::{self, Artwork};
use resources::definition::{self, Definition};
use resources::quote::{self, Quote};
use resources::scripture::{self, Scripture};

use std::path::Path;
use std::sync::Arc;

use askama::Template;
use axum::{
    Form, Router,
    body::Body,
    extract::{self, Query, Request, State},
    http::{HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use sqlx::SqlitePool;
use tower::ServiceExt;
use tower_http::services::ServeFile;

#[derive(Clone)]
struct AppState {
    pool:       SqlitePool,
    images_dir: Arc<Path>,
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

/// All four display pages in one document; the quote is shown first.
#[derive(Template)]
#[template(path = "display.html")]
struct DisplayTemplate {
    quote:     Option<Quote>,
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
struct LinkForm {
    link: String,
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
    #[serde(default, deserialize_with = "empty_as_none")]
    verse_end:   Option<u16>,
    text:        String,
}

/// An optional number input left blank is submitted as an empty string,
/// which would otherwise fail to parse and reject the whole form.
fn empty_as_none<'de, D, T>(de: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    let s: Option<String> = serde::Deserialize::deserialize(de)?;
    match s.as_deref().map(str::trim) {
        None | Some("") => Ok(None),
        Some(s)         => s.parse().map(Some).map_err(serde::de::Error::custom),
    }
}

#[tokio::main]
async fn main() {
    let config = Config::from_env();
    let pool = db::connect(&config).await.unwrap_or_else(|e| {
        eprintln!("failed to open database {}: {e}", config.database_path().display());
        std::process::exit(1);
    });

    let state = AppState { pool, images_dir: config.images_dir().into() };
    println!("Server listening on http://{}", config.bind);

    let app = Router::new()
        .route("/",                          get(display))
        .route("/api/wotd",                  get(api_wotd))
        .route("/reroll/{page}",             post(reroll))
        .route("/favicon.svg",               get(favicon))
        .route("/images/{id}/{name}",        get(image))
        .route("/thumbs/{id}",               get(thumb))
        .route("/admin",                     get(admin_get))
        .route("/admin/quotes",              post(admin_quotes_post))
        .route("/admin/quotes/delete",       post(admin_quotes_delete))
        .route("/admin/artworks",            post(admin_artworks_post))
        .route("/admin/artworks/link",       post(admin_artworks_link))
        .route("/admin/artworks/delete",     post(admin_artworks_delete))
        .route("/admin/definitions",         post(admin_definitions_post))
        .route("/admin/definitions/delete",  post(admin_definitions_delete))
        .route("/admin/scriptures",          post(admin_scriptures_post))
        .route("/admin/scriptures/link",     post(admin_scriptures_link))
        .route("/admin/scriptures/delete",   post(admin_scriptures_delete))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(&config.bind).await.unwrap_or_else(|e| {
        eprintln!("failed to bind {}: {e}", config.bind);
        std::process::exit(1);
    });
    let _ = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await;
}

/// Resolves on Ctrl+C or SIGTERM, which `docker stop` sends on every redeploy.
/// Without it the server ignores SIGTERM and is killed after a timeout.
async fn shutdown_signal() {
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sigterm) => { sigterm.recv().await; }
            Err(_)          => std::future::pending().await,
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = terminate => {}
    }
}

// ── Display handlers ──────────────────────────────────────────────────────────

/// The display page; the arrow keys switch between its four parts in place.
async fn display(State(state): State<AppState>) -> HandlerResult<Html<String>> {
    Ok(Html(DisplayTemplate {
        quote:     daily::todays(&state.pool, Kind::Quote).await?,
        artwork:   daily::todays(&state.pool, Kind::Artwork).await?,
        word_en:   daily::todays(&state.pool, Kind::WordEn).await?,
        word_fr:   daily::todays(&state.pool, Kind::WordFr).await?,
        scripture: daily::todays(&state.pool, Kind::Scripture).await?,
    }.render()?))
}

/// Today's two words as JSON, for the Android live wallpaper.
/// `next_rollover` says when they change, barring a manual reroll.
async fn api_wotd(State(state): State<AppState>) -> HandlerResult<Response> {
    let word = |d: Option<Definition>| {
        d.map(|d| serde_json::json!({
            "word":        d.word,
            "translation": d.translation,
            "definition":  d.definition,
        }))
    };
    let body = serde_json::json!({
        "day":           daily::today().to_string(),
        "next_rollover": daily::next_rollover().to_rfc3339(),
        "french":        word(daily::todays(&state.pool, Kind::WordFr).await?),
        "english":       word(daily::todays(&state.pool, Kind::WordEn).await?),
    });
    Ok(([(header::CACHE_CONTROL, "no-store")], axum::Json(body)).into_response())
}

/// Replace today's pick(s) shown on a display page; its "r" key calls this.
async fn reroll(
    State(state): State<AppState>,
    extract::Path(page): extract::Path<String>,
) -> HandlerResult<StatusCode> {
    let kinds: &[Kind] = match page.as_str() {
        "quote"     => &[Kind::Quote],
        "artwork"   => &[Kind::Artwork],
        "words"     => &[Kind::WordEn, Kind::WordFr],
        "scripture" => &[Kind::Scripture],
        _           => return Ok(StatusCode::NOT_FOUND),
    };
    for &kind in kinds {
        daily::reroll(&state.pool, kind).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── Images ────────────────────────────────────────────────────────────────────

/// A gilded fleur-de-lis, compiled into the binary.
async fn favicon() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "image/svg+xml"), (header::CACHE_CONTROL, "public, max-age=86400")],
        include_str!("../static/favicon.svg"),
    )
}

/// Full-size artwork image. `name` is only there so browsers save the file
/// under the artwork's name; Content-Disposition says the same for those
/// that look at it.
async fn image(
    State(state): State<AppState>,
    extract::Path((id, _name)): extract::Path<(i64, String)>,
    req: Request,
) -> HandlerResult<Response> {
    let Some(a) = resources::get::<Artwork>(&state.pool, id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    let mut res = serve_file(&a.image_path(&state.images_dir), req).await;
    if res.status().is_success() {
        let name = a.download_name();
        let ascii: String = name.chars().map(|c| if c.is_ascii() { c } else { '_' }).collect();
        let disposition = format!(
            "inline; filename=\"{ascii}\"; filename*=UTF-8''{}",
            urlencoding::encode(&name),
        );
        if let Ok(value) = HeaderValue::from_str(&disposition) {
            res.headers_mut().insert(header::CONTENT_DISPOSITION, value);
        }
    }
    Ok(res)
}

async fn thumb(
    State(state): State<AppState>,
    extract::Path(id): extract::Path<i64>,
    req: Request,
) -> HandlerResult<Response> {
    let Some(a) = resources::get::<Artwork>(&state.pool, id).await? else {
        return Ok(StatusCode::NOT_FOUND.into_response());
    };
    Ok(serve_file(&a.thumb_path(&state.images_dir), req).await)
}

/// Serve `path` (with range and conditional request support). Successful
/// responses are cached forever: an artwork id never gets a different image.
async fn serve_file(path: &Path, req: Request) -> Response {
    let Ok(res) = ServeFile::new(path).oneshot(req).await;
    let mut res = res.map(Body::new);
    if res.status().is_success() {
        res.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    }
    res
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
    let new = artwork::NewArtwork {
        url:    form.url.trim(),
        artist: form.artist.trim(),
        work:   form.work.trim(),
        date:   form.date.trim(),
    };
    match artwork::download_and_add(&state.pool, &state.images_dir, &new).await {
        Ok(())  => Redirect::to("/admin#artworks"),
        Err(e)  => err_redirect("artworks", e),
    }
}

/// One step: look up the Wikipedia/Wikidata link, download the image, store it.
async fn admin_artworks_link(State(state): State<AppState>, Form(form): Form<LinkForm>) -> Redirect {
    match artwork::add_from_link(&state.pool, &state.images_dir, &form.link).await {
        Ok(())  => Redirect::to("/admin#artworks"),
        Err(e)  => err_redirect("artworks", e),
    }
}

async fn admin_artworks_delete(State(state): State<AppState>, Form(form): Form<DeleteForm>) -> HandlerResult<Redirect> {
    artwork::delete(&state.pool, &state.images_dir, form.id).await?;
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

/// One step: parse the BibleGateway link, fetch the KJV text, store it.
async fn admin_scriptures_link(State(state): State<AppState>, Form(form): Form<LinkForm>) -> Redirect {
    let result = async {
        let s = scripture::from_link(&form.link).await?;
        scripture::add(&state.pool, &s).await
    }.await;
    match result {
        Ok(())  => Redirect::to("/admin#scriptures"),
        Err(e)  => err_redirect("scriptures", e),
    }
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
