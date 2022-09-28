use std::{
    io::{Cursor, Write},
    path::PathBuf,
};

use anyhow::Context;
use tracing::info;
use tracing::warn;

use crate::{BeatmapUrlProvider, HTTP_CLIENT};

pub static MAP_DIRECTORY: &str = "osu_maps";

fn map_directory() -> PathBuf {
    let mut dir = PathBuf::from(".");
    dir.push(MAP_DIRECTORY);
    dir
}

pub fn find_non_downloaded_maps(beatmap_ids: &[u64]) -> anyhow::Result<Vec<u64>> {
    let downloaded_maps_iter =
        std::fs::read_dir(map_directory()).context("Error while reading map directory.")?;
    let downloaded_map_ids: Vec<_> = downloaded_maps_iter
        .filter_map(|dir_entry_res| dir_entry_res.ok())
        .filter_map(|dir_entry| dir_entry.file_name().into_string().ok())
        .collect();
    warn!("Downloaded maps {:?}", downloaded_map_ids);

    let mut absent_maps = Vec::new();

    for beatmap_id in beatmap_ids {
        let beatmap_file_name = beatmap_id.to_string() + ".osz";
        if !downloaded_map_ids.contains(&beatmap_file_name) {
            info!(beatmap_id, "Map `{beatmap_id}` is not downloaded.");
            absent_maps.push(*beatmap_id);
        }
    }

    info!("Total {} maps not found.", absent_maps.len());
    Ok(absent_maps)
}

pub async fn download_map(beatmap_id: u64) -> anyhow::Result<()> {
    let mut url_provider = BeatmapUrlProvider::new(beatmap_id);

    loop {
        let response = HTTP_CLIENT.get(url_provider.get_next_url()?).send().await;
        info!("Downloading map {}", beatmap_id);

        if response.is_err() {
            info!(
                "Error returned while downloading map `{}`. Trying the next mirror.",
                beatmap_id
            );
            continue;
        }
        let response = response.unwrap();

        if !response.status().is_success() {
            info!(
                "Mirror returned {}. Trying the next mirror.",
                response.status()
            );
            continue;
        }

        let mut file_path = map_directory();
        file_path.push(beatmap_id.to_string());
        file_path.set_extension("osz");
        let mut content = Cursor::new(
            response
                .bytes()
                .await
                .context("Unable to convert body to bytes.")?,
        );

        // We check if another downloader task (spawned from another request) already created the file.
        // If so do not write to it.
        // TODO!: Move file writes to a worker task.
        if file_path.exists() {
            info!(
                "Map `{}` already exists in the file system. Download yielded to other task.",
                beatmap_id
            );
            return Ok(());
        }

        tokio::task::spawn_blocking(move || {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(file_path.clone())
                .context("Unable to create map file.")?;

            let copy_res =
                std::io::copy(&mut content, &mut file).context("Unable to write map data to file.");

            if copy_res.is_err() {
                info!(
                    "Unable to copy beatmap `{}` to file. Deleting artifact.",
                    beatmap_id
                );
                std::fs::remove_file(file_path).context("Unable to delete the empty file.")?;
            }

            copy_res
        })
        .await??;

        break;
    }

    Ok(())
}

pub fn zip_beatmaps(beatmap_ids: &[u64]) -> anyhow::Result<Vec<u8>> {
    info!("Zipping {} beatmaps.", beatmap_ids.len());
    let buffer = Vec::new();
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(buffer));
    let options =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);

    for beatmap_id in beatmap_ids {
        info!("Adding `{}` to zip file.", beatmap_id);
        zip.start_file(beatmap_id.to_string() + ".osz", options)?;
        let mut file_path = map_directory();
        file_path.push(beatmap_id.to_string());
        file_path.set_extension("osz");

        let beatmap_file = std::fs::read(file_path)?;
        zip.write_all(&beatmap_file)?;
        info!("Added `{}` to zip file.", beatmap_id);
    }

    Ok(zip.finish()?.into_inner())
}
