use axum::{
    extract::Json,
    http::{header, HeaderMap},
    response::IntoResponse,
    Extension,
};
use futures::future::try_join_all;
use serde::{Deserialize, Deserializer};
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::{
    beatmap_store::{download_map, find_non_downloaded_maps, zip_beatmaps},
    osu_api::get_beatmap_id_from_diff_ids,
    AppError, AppState,
};

#[derive(Deserialize)]
pub struct ServeMapsRequest {
    maps: Vec<u64>,
    #[serde(deserialize_with = "id_type_from_string")]
    id_type: IdType,
}

enum IdType {
    Beatmap,
    Difficulty,
}

fn id_type_from_string<'de, D>(deserializer: D) -> Result<IdType, D::Error>
where
    D: Deserializer<'de>,
{
    let s: &str = Deserialize::deserialize(deserializer)?;
    match s {
        "beatmap" => Ok(IdType::Beatmap),
        "difficulty" => Ok(IdType::Difficulty),
        _ => Ok(IdType::Beatmap),
    }
}

pub async fn serve_maps(
    Json(request): Json<ServeMapsRequest>,
    Extension(state): Extension<Arc<RwLock<AppState>>>,
) -> Result<impl IntoResponse, AppError> {
    let mut map_list = match request.id_type {
        IdType::Beatmap => request.maps,
        IdType::Difficulty => get_beatmap_id_from_diff_ids(&request.maps, state).await?,
    };
    map_list.sort_unstable();
    map_list.dedup();

    let absent_maps = find_non_downloaded_maps(&map_list)?;

    if !absent_maps.is_empty() {
        let mut download_futures = Vec::new();

        for map_id in absent_maps {
            download_futures.push(download_map(map_id));
        }

        try_join_all(download_futures).await?;
    }

    let zipped_maps = tokio::task::spawn_blocking(move || zip_beatmaps(&map_list)).await??;

    Ok(return_zip_file_with_headers(zipped_maps))
}

fn return_zip_file_with_headers(data: Vec<u8>) -> impl IntoResponse {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, "application/zip".parse().unwrap());
    headers.insert(
        header::CONTENT_DISPOSITION,
        "attachment; filename=\"maps.zip\"".parse().unwrap(),
    );
    (headers, data)
}
