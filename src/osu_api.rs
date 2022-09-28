use std::{sync::Arc, time::Duration};

use anyhow::{bail, Context};
use bonsaidb::core::keyvalue::AsyncKeyValue;
use futures::future::try_join_all;
use reqwest::StatusCode;
use tokio::sync::RwLock;
use tracing::{debug, error, info, instrument};

use crate::{AppState, HTTP_CLIENT};

const OSU_API_ENDPOINT: &str = "https://osu.ppy.sh/api/v2/beatmaps/";
const OSU_OAUTH_ENDPOINT: &str = "https://osu.ppy.sh/oauth/token";

pub fn fetch_beatmap_id_url(beatmap_id: u64) -> String {
    OSU_API_ENDPOINT.to_string() + &beatmap_id.to_string()
}

#[instrument]
pub async fn get_osu_access_token() -> anyhow::Result<(String, u64)> {
    info!("Getting new osu access token.");
    let client_id = std::env::var("OSU_CLIENT_ID").expect("OSU_CLIENT_ID env var is not set!");
    let client_secret =
        std::env::var("OSU_CLIENT_SECRET").expect("OSU_CLIENT_SECRET env var is not set!");

    let params = [
        ("client_id", client_id),
        ("client_secret", client_secret),
        ("grant_type", "client_credentials".to_string()),
        ("scope", "public".to_string()),
    ];

    let response: serde_json::Value = HTTP_CLIENT
        .post(OSU_OAUTH_ENDPOINT)
        .form(&params)
        .send()
        .await
        .context("Unable to fetch access token from Osu! API")?
        .json()
        .await?;

    let response = response
        .as_object()
        .context("Return value is not a valid JSON!")?;

    let access_token = response
        .get("access_token")
        .context("Return value does not have \"access_token\"")?
        .as_str()
        .expect("Converting access_token into string should not fail.")
        .to_string();

    let expires_in = response
        .get("expires_in")
        .context("Return value does not have \"expires_in\"")?
        .as_u64()
        .expect("Converting expires_in into a number should not fail.");

    info!("Successfully got osu access token.");
    Ok((access_token, expires_in))
}

pub async fn get_beatmap_id_from_diff_ids(
    difficulty_ids: &[u64],
    state: Arc<RwLock<AppState>>,
) -> anyhow::Result<Vec<u64>> {
    if difficulty_ids.is_empty() {
        error!("Difficulty ID list is empty.");
        bail!("Difficulty Id list is empty.")
    }

    let (mut beatmap_ids, difficulty_ids) =
        get_beatmap_ids_from_db(difficulty_ids, state.clone()).await;

    let beatmap_id_futures: Vec<_> = difficulty_ids
        .iter()
        .map(|diff_id| async {
            info!(diff_id = *diff_id, "Fetching beatmap id from osu API");
            let response = HTTP_CLIENT
                .get(fetch_beatmap_id_url(*diff_id))
                .bearer_auth(state.read().await.osu_access_token.clone())
                .send()
                .await?;

            if response.status() != StatusCode::OK {
                bail!("Failed to fetch beatmap ID from difficulty ID.")
            }

            let beatmap_id = response
                .json::<serde_json::Value>()
                .await?
                .as_object()
                .expect("Converting successful beatmap response should not fail.")
                .get("beatmapset_id")
                .expect("Successful beatmap request should have \"beatmapset_id\" field.")
                .as_u64()
                .expect("\"beatmapset_id\" must be a Number");

            info!(
                beatmap_id,
                diff_id = *diff_id,
                "Saving beatmap ID for the diff to the database."
            );
            state
                .read()
                .await
                .db
                .set_key(diff_id.to_string(), &beatmap_id)
                .await
                .context("Error occured writing beatmap_id to database")?;

            Ok(beatmap_id)
        })
        .collect();

    let mut future_result = try_join_all(beatmap_id_futures).await?;
    beatmap_ids.append(&mut future_result);
    Ok(beatmap_ids)
}

async fn get_beatmap_ids_from_db(
    difficulty_ids: &[u64],
    state: Arc<RwLock<AppState>>,
) -> (Vec<u64>, Vec<u64>) {
    let mut beatmap_ids = Vec::new();
    let mut unknown_difficulty_ids = Vec::new();
    let mut difficulty_ids = Vec::from(difficulty_ids);
    difficulty_ids.sort();
    difficulty_ids.dedup();

    info!(
        num_of_ids_to_resolve = difficulty_ids.len(),
        "Trying to resolve difficulty ids. {:?}", difficulty_ids
    );

    for diff_id in &difficulty_ids {
        match state
            .read()
            .await
            .db
            .get_key(diff_id.to_string())
            .into::<u64>()
            .await
            .expect("Database can't hold non-u64 values")
        {
            Some(beatmap_id) => {
                info!(diff_id, beatmap_id, "Found entry for difficulty id.",);
                beatmap_ids.push(beatmap_id)
            }
            None => {
                debug!(diff_id, "Couldn't find entry for difficulty id.");
                unknown_difficulty_ids.push(*diff_id)
            }
        }
    }

    info!(
        "Found {} map entries out of {}.",
        beatmap_ids.len(),
        difficulty_ids.len()
    );
    (beatmap_ids, unknown_difficulty_ids)
}

pub async fn refresh_token_periodically(state: Arc<RwLock<AppState>>) -> anyhow::Result<()> {
    loop {
        let mut write_lock = state.write().await;
        let access_token_response = get_osu_access_token().await;
        if let Err(err) = access_token_response {
            println!("{err}");
            std::process::exit(31);
        }
        let (access_token, expires_in) = access_token_response?;
        write_lock.osu_access_token = access_token;
        drop(write_lock);
        tokio::time::sleep(Duration::from_secs(expires_in - 3600)).await;
    }
}
