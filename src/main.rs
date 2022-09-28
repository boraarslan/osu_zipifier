use std::sync::Arc;

use bonsaidb::local::{
    config::{Builder, StorageConfiguration},
    AsyncDatabase,
};
use tokio::sync::RwLock;

use axum::{routing::get, Extension};
use osu_zipifier::{
    beatmap_store::MAP_DIRECTORY, osu_api::refresh_token_periodically, routes::serve_maps, AppState,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    setup_logging();
    if std::fs::read_dir(MAP_DIRECTORY).is_err() {
        std::fs::create_dir(MAP_DIRECTORY).unwrap()
    }
    dotenvy::dotenv().unwrap();
    let db = AsyncDatabase::open::<()>(StorageConfiguration::new("diff-beatmap.bonsaidb")).await?;
    let shared_state = Arc::new(RwLock::new(AppState {
        osu_access_token: String::new(),
        db,
    }));

    tokio::task::spawn(refresh_token_periodically(shared_state.clone()));

    let app = axum::Router::new()
        .route("/", get(serve_maps))
        .layer(Extension(shared_state));

    axum::Server::bind(&"0.0.0.0:3000".parse().unwrap())
        .serve(app.into_make_service())
        .await
        .unwrap();

    Ok(())
}

fn setup_logging() {
    tracing_subscriber::fmt::fmt().init();
}
