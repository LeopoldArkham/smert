mod error;
mod resources;

use resources::artwork::{self, Artwork};
use resources::definition::{self, Definition};
use resources::quote::{self, Quote};
use resources::scripture::{self, Scripture};
use resources::DailyResource;

use std::sync::{Arc, RwLock};
use std::time::Duration;

use askama::Template;
use axum::{
    Form, Router,
    extract::{Query, State},
    http::StatusCode,
    response::{Html, Redirect},
    routing::{get, post},
};
use clokwerk::{Job, Scheduler, TimeUnits};
use tower_http::services::ServeDir;

#[derive(Clone)]
struct AppState {
    quotes:     Arc<RwLock<Vec<Quote>>>,
    artworks:   Arc<RwLock<Vec<Artwork>>>,
    words_en:   Arc<RwLock<Vec<Definition>>>,
    words_fr:   Arc<RwLock<Vec<Definition>>>,
    scriptures: Arc<RwLock<Vec<Scripture>>>,
}

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
    definitions_en_count: usize,
    definitions_fr:       Vec<Definition>,
    definitions_fr_count: usize,
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
    index: usize,
}

#[derive(serde::Deserialize)]
struct DeleteDefinitionForm {
    index: usize,
    lang:  String,
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

    let quotes     = quote::load_quotes().expect("failed to load quotes");
    let artworks   = artwork::load_artworks().unwrap_or_default();
    let defs_en    = definition::load_definitions("english").unwrap_or_default();
    let defs_fr    = definition::load_definitions("french").unwrap_or_default();
    let scriptures = scripture::load_scriptures().unwrap_or_default();

    let state = AppState {
        quotes:     Arc::new(RwLock::new(Quote::pick_n(&quotes, 1))),
        artworks:   Arc::new(RwLock::new(Artwork::pick_n(&artworks, 1))),
        words_en:   Arc::new(RwLock::new(definition::pick_one(&defs_en))),
        words_fr:   Arc::new(RwLock::new(definition::pick_one(&defs_fr))),
        scriptures: Arc::new(RwLock::new(Scripture::pick_n(&scriptures, 1))),
    };

    let mut scheduler = Scheduler::new();

    // All resources refresh daily at 05:00.
    let s = state.clone();
    scheduler.every(1.day()).at("05:00").run(move || refresh_all(&s));

    let _scheduler_handle = scheduler.watch_thread(Duration::from_millis(500));

    // Press "r" + Enter in the console to re-roll every random selection on demand.
    let stdin_state = state.clone();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            match stdin.read_line(&mut line) {
                Ok(0) => break, // EOF — stdin closed
                Ok(_) => {
                    if line.trim().eq_ignore_ascii_case("r") {
                        refresh_all(&stdin_state);
                        println!("↻ Reloaded random selections.");
                    }
                }
                Err(_) => break,
            }
        }
    });
    println!("Server listening on http://0.0.0.0:1212  (press \"r\" + Enter to reshuffle)");

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

    let listener = tokio::net::TcpListener::bind("0.0.0.0:1212").await.unwrap();
    let _ = axum::serve(listener, app).await;
}

/// Reload every resource from disk and re-roll its random daily selection.
/// Shared by the daily scheduler and the console "r" reload command.
fn refresh_all(state: &AppState) {
    if let Ok(all) = quote::load_quotes() {
        *state.quotes.write().unwrap() = Quote::pick_n(&all, 1);
    }
    if let Ok(all) = artwork::load_artworks() {
        *state.artworks.write().unwrap() = Artwork::pick_n(&all, 1);
    }
    if let Ok(all) = definition::load_definitions("english") {
        *state.words_en.write().unwrap() = definition::pick_one(&all);
    }
    if let Ok(all) = definition::load_definitions("french") {
        *state.words_fr.write().unwrap() = definition::pick_one(&all);
    }
    if let Ok(all) = scripture::load_scriptures() {
        *state.scriptures.write().unwrap() = Scripture::pick_n(&all, 1);
    }
}

// ── Display handlers ──────────────────────────────────────────────────────────

async fn qotd(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    let quotes = state.quotes.read().unwrap();
    let q = quotes.first().expect("quote selection is never empty");
    (StatusCode::OK, Html(quote::render_quote(q).unwrap()))
}

async fn artwork_handler(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    let artworks = state.artworks.read().unwrap();
    match artworks.first() {
        Some(a) => (StatusCode::OK, Html(artwork::render_artwork(a).unwrap())),
        None    => (StatusCode::NOT_FOUND, Html("<p>No artwork yet.</p>".into())),
    }
}

async fn wotd(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    let word_en = state.words_en.read().unwrap().first().cloned();
    let word_fr = state.words_fr.read().unwrap().first().cloned();
    let tmpl = WotdTemplate { word_en, word_fr };
    match tmpl.render() {
        Ok(html) => (StatusCode::OK, Html(html)),
        Err(_)   => (StatusCode::INTERNAL_SERVER_ERROR, Html("Template error".into())),
    }
}

async fn home(State(state): State<AppState>) -> (StatusCode, Html<String>) {
    let quote     = state.quotes.read().unwrap().first().cloned()
                      .expect("quote selection is never empty");
    let artwork   = state.artworks.read().unwrap().first().cloned();
    let word_en   = state.words_en.read().unwrap().first().cloned();
    let word_fr   = state.words_fr.read().unwrap().first().cloned();
    let scripture = state.scriptures.read().unwrap().first().cloned();
    let tmpl = HomeTemplate { quote, artwork, word_en, word_fr, scripture };
    match tmpl.render() {
        Ok(html) => (StatusCode::OK,                    Html(html)),
        Err(_)   => (StatusCode::INTERNAL_SERVER_ERROR, Html("Template error".into())),
    }
}

// ── Admin handlers ────────────────────────────────────────────────────────────

fn err_redirect(tab: &str, e: impl std::fmt::Display) -> Redirect {
    Redirect::to(&format!(
        "/admin?error={}#{}",
        urlencoding::encode(&e.to_string()),
        tab,
    ))
}

async fn admin_get(Query(query): Query<AdminQuery>) -> (StatusCode, Html<String>) {
    let quotes         = quote::load_quotes().unwrap_or_default();
    let artworks       = artwork::load_artworks().unwrap_or_default();
    let definitions_en = definition::load_definitions("english").unwrap_or_default();
    let definitions_fr = definition::load_definitions("french").unwrap_or_default();
    let scriptures     = scripture::load_scriptures().unwrap_or_default();
    let authors        = quote::unique_authors(&quotes);
    let works          = quote::unique_works(&quotes);
    let artists        = artwork::unique_artists(&artworks);
    let quotes_json    = serde_json::to_string(&quotes).unwrap_or_default();
    let definition_count     = definitions_en.len() + definitions_fr.len();
    let definitions_en_count = definitions_en.len();
    let definitions_fr_count = definitions_fr.len();
    let quote_count          = quotes.len();
    let artwork_count        = artworks.len();
    let scripture_count      = scriptures.len();

    let mut quotes_rev = quotes;
    quotes_rev.reverse();
    let mut artworks_rev = artworks;
    artworks_rev.reverse();
    let mut defs_en_rev = definitions_en;
    defs_en_rev.reverse();
    let mut defs_fr_rev = definitions_fr;
    defs_fr_rev.reverse();
    let mut scriptures_rev = scriptures;
    scriptures_rev.reverse();

    let tmpl = AdminTemplate {
        quote_count,
        quotes: quotes_rev,
        authors,
        works,
        quotes_json,
        artwork_count,
        artworks: artworks_rev,
        artists,
        definition_count,
        definitions_en: defs_en_rev,
        definitions_en_count,
        definitions_fr: defs_fr_rev,
        definitions_fr_count,
        scripture_count,
        scriptures: scriptures_rev,
        error: query.error,
    };
    match tmpl.render() {
        Ok(html) => (StatusCode::OK,                    Html(html)),
        Err(_)   => (StatusCode::INTERNAL_SERVER_ERROR, Html("Template error".into())),
    }
}

async fn admin_quotes_post(Form(form): Form<NewQuoteForm>) -> Redirect {
    let work = (!form.work.trim().is_empty()).then(|| form.work.trim().to_string());
    let date = (!form.date.trim().is_empty()).then(|| form.date.trim().to_string());
    match quote::add_quote(Quote {
        text:     form.text.trim().to_string(),
        author:   form.author.trim().to_string(),
        work,
        date,
        language: form.language,
    }) {
        Ok(())  => Redirect::to("/admin"),
        Err(e)  => err_redirect("quotes", e),
    }
}

async fn admin_quotes_delete(Form(form): Form<DeleteForm>) -> Redirect {
    let _ = quote::delete_quote(form.index);
    Redirect::to("/admin")
}

async fn admin_artworks_post(Form(form): Form<NewArtworkForm>) -> Redirect {
    match artwork::download_and_add(
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

async fn admin_artworks_delete(Form(form): Form<DeleteForm>) -> Redirect {
    let _ = artwork::delete_artwork(form.index);
    Redirect::to("/admin#artworks")
}

async fn admin_definitions_post(Form(form): Form<NewDefinitionForm>) -> Redirect {
    let fr = form.word_fr.trim().to_string();
    let en = form.word_en.trim().to_string();
    let mut errors: Vec<String> = Vec::new();
    let fr_fut = async {
        if !fr.is_empty() {
            if let Err(e) = definition::fetch_and_add(&fr, "french").await {
                Some(format!("fr: {e}"))
            } else { None }
        } else { None }
    };
    let en_fut = async {
        if !en.is_empty() {
            if let Err(e) = definition::fetch_and_add(&en, "english").await {
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

async fn admin_definitions_delete(Form(form): Form<DeleteDefinitionForm>) -> Redirect {
    let _ = definition::delete_definition(&form.lang, form.index);
    Redirect::to("/admin#definitions")
}

async fn admin_scriptures_post(Form(form): Form<NewScriptureForm>) -> Redirect {
    let verse_end = form.verse_end.filter(|&v| v > 0 && v != form.verse_start);
    match scripture::add_scripture(Scripture {
        book:        form.book.trim().to_string(),
        chapter:     form.chapter,
        verse_start: form.verse_start,
        verse_end,
        text:        form.text.trim().to_string(),
    }) {
        Ok(())  => Redirect::to("/admin#scriptures"),
        Err(e)  => err_redirect("scriptures", e),
    }
}

async fn admin_scriptures_delete(Form(form): Form<DeleteForm>) -> Redirect {
    let _ = scripture::delete_scripture(form.index);
    Redirect::to("/admin#scriptures")
}
