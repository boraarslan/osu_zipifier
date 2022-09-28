use anyhow::Context;
use axum::response::{IntoResponse, Response};
use bonsaidb::local::AsyncDatabase;
use once_cell::sync::Lazy;
use strum::{EnumIter, IntoEnumIterator};

use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::{policies::ExponentialBackoff, RetryTransientMiddleware};

pub mod beatmap_store;
pub mod osu_api;
pub mod routes;

#[derive(Clone)]
pub struct AppState {
    pub osu_access_token: String,
    pub db: AsyncDatabase,
}

pub static HTTP_CLIENT: Lazy<ClientWithMiddleware> = Lazy::new(|| {
    let retry_policy = ExponentialBackoff::builder()
        .backoff_exponent(2)
        .build_with_max_retries(3);

    ClientBuilder::new(reqwest::Client::new())
        .with(RetryTransientMiddleware::new_with_policy(retry_policy))
        .build()
});

pub struct BeatmapUrlProvider {
    pub beatmap_id: u64,
    endpoint: BeatmapEndpointIter,
}

impl BeatmapUrlProvider {
    pub fn new(beatmap_id: u64) -> Self {
        Self {
            beatmap_id,
            endpoint: BeatmapEndpoint::iter(),
        }
    }

    pub fn get_next_url(&mut self) -> anyhow::Result<String> {
        Ok(self
            .endpoint
            .next()
            .context("Out of backup endpoints")?
            .get_download_url(self.beatmap_id))
    }
}

/// Beatmap endpoint list.
/// Since this implements Iterator, it is also the priority queue for the mirror list.
#[derive(EnumIter)]
enum BeatmapEndpoint {
    Catboy,
    Chimu,
    Nerinyan,
}

impl BeatmapEndpoint {
    pub fn get_download_url(&self, beatmap_id: u64) -> String {
        match self {
            BeatmapEndpoint::Chimu => "https://chimu.moe/d/".to_string() + &beatmap_id.to_string(),
            BeatmapEndpoint::Catboy => {
                "https://catboy.best/d/".to_string() + &beatmap_id.to_string()
            }
            BeatmapEndpoint::Nerinyan => {
                "https://proxy.nerinyan.moe/d/".to_string() + &beatmap_id.to_string()
            }
        }
    }
}

// Make our own error that wraps `anyhow::Error`.
pub struct AppError(anyhow::Error);

// Tell axum how to convert `AppError` into a response.
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            format!("Something went wrong: {}", self.0),
        )
            .into_response()
    }
}

// This enables using `?` on functions that return `Result<_, anyhow::Error>` to turn them into
// `Result<_, AppError>`. That way you don't need to do that manually.
impl<E> From<E> for AppError
where
    E: Into<anyhow::Error>,
{
    fn from(err: E) -> Self {
        Self(err.into())
    }
}
