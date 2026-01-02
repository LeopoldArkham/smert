mod error;
mod quote;

use axum::{Router, http::StatusCode, response::Html, routing::get};

#[tokio::main]
async fn main() {
    let app = Router::new().route("/qotd", get(qotd));

    let listener = tokio::net::TcpListener::bind("0.0.0.0:1212").await.unwrap();
    let _ = axum::serve(listener, app).await; // Handle
}

async fn qotd() -> (StatusCode, Html<String>) {
    let quote = quote::get_current_quote().unwrap();
    (StatusCode::OK, Html(quote::render_quote(&quote).unwrap()))
}

// Restore my nice error handling with ?
