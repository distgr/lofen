/* --------------------------
Local file scanner
    - Scans configured music_paths for audio files
    - Reads metadata tags with lofty
    - Inserts/updates artists, albums, tracks in the SQLite database
-------------------------- */
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::Accessor;
use sqlx::{Pool, Sqlite};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use walkdir::WalkDir;

const AUDIO_EXTENSIONS: &[&str] = &[
    "mp3", "flac", "ogg", "opus", "m4a", "aac", "wav", "wv", "ape", "mpc", "mp4", "aiff", "aif",
];

fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

fn path_id(path: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    path.hash(&mut h);
    format!("local-{:016x}", h.finish())
}

fn read_tag_string(tag: &lofty::tag::Tag, item_key: lofty::tag::ItemKey) -> String {
    tag.get_string(&item_key).unwrap_or("").to_string()
}

pub struct ScanStats {
    pub scanned: usize,
    pub inserted: usize,
    pub errors: usize,
}

pub async fn scan_paths(
    pool: &Arc<Pool<Sqlite>>,
    paths: &[String],
) -> Result<ScanStats, Box<dyn std::error::Error + Send + Sync>> {
    let mut stats = ScanStats { scanned: 0, inserted: 0, errors: 0 };

    // Collect all audio files first
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for root in paths {
        for entry in WalkDir::new(root).follow_links(true).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path().to_path_buf();
            if path.is_file() && is_audio(&path) {
                files.push(path);
            }
        }
    }

    // Group by album to build Artist/Album records
    // album_key -> (album_artist, album_name, Vec<track_path>)
    let mut album_map: HashMap<String, (String, String, Vec<std::path::PathBuf>)> = HashMap::new();

    for file in &files {
        stats.scanned += 1;
        let path_str = file.to_string_lossy().to_string();

        let tagged = match lofty::read_from_path(file) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("Failed to read tags from {}: {}", path_str, e);
                stats.errors += 1;
                continue;
            }
        };

        let tag = match tagged.primary_tag().or_else(|| tagged.first_tag()) {
            Some(t) => t,
            None => {
                // No tags: use filename as title
                let title = file.file_stem().unwrap_or_default().to_string_lossy().to_string();
                let album_artist = "Unknown Artist".to_string();
                let album_name = "Unknown Album".to_string();
                let key = path_id(&format!("{}{}", album_artist, album_name));
                album_map
                    .entry(key)
                    .or_insert_with(|| (album_artist, album_name, Vec::new()))
                    .2
                    .push(file.clone());
                continue;
            }
        };

        let album_artist = {
            let aa = read_tag_string(tag, lofty::tag::ItemKey::AlbumArtist);
            if aa.is_empty() {
                read_tag_string(tag, lofty::tag::ItemKey::TrackArtist)
            } else {
                aa
            }
        };
        let album_artist = if album_artist.is_empty() { "Unknown Artist".to_string() } else { album_artist };
        let album_name = {
            let a = read_tag_string(tag, lofty::tag::ItemKey::AlbumTitle);
            if a.is_empty() { "Unknown Album".to_string() } else { a }
        };
        let album_key = path_id(&format!("{}{}", album_artist, album_name));
        album_map
            .entry(album_key)
            .or_insert_with(|| (album_artist, album_name, Vec::new()))
            .2
            .push(file.clone());
    }

    let mut tx = pool.begin().await?;

    for (album_key, (album_artist, album_name, track_files)) in &album_map {
        // Upsert artist
        let artist_id = path_id(&album_artist.to_ascii_lowercase());
        let artist_json = serde_json::json!({
            "Id": artist_id,
            "Name": album_artist,
            "RunTimeTicks": 0,
            "Type": "",
            "UserData": {},
            "DateCreated": ""
        })
        .to_string();
        sqlx::query(
            "INSERT INTO artists (id, artist) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET artist = excluded.artist",
        )
        .bind(&artist_id)
        .bind(&artist_json)
        .execute(&mut *tx)
        .await?;

        // Upsert album
        let album_json = serde_json::json!({
            "Id": album_key,
            "Name": album_name,
            "AlbumArtists": [{"Id": artist_id, "Name": album_artist}],
            "UserData": {},
            "DateCreated": "",
            "ParentId": "",
            "RunTimeTicks": 0,
            "ProductionYear": 0,
            "PremiereDate": ""
        })
        .to_string();
        sqlx::query(
            r#"INSERT INTO albums (id, album) VALUES (?, ?)
               ON CONFLICT(id) DO UPDATE SET album = excluded.album"#,
        )
        .bind(&album_key)
        .bind(&album_json)
        .execute(&mut *tx)
        .await?;

        // Upsert tracks
        for file in track_files {
            let path_str = file.to_string_lossy().to_string();
            let track_id = path_id(&path_str);

            let tagged = match lofty::read_from_path(file) {
                Ok(t) => t,
                Err(_) => continue,
            };

            let props = tagged.properties();
            let duration_secs = props.duration().as_secs_f64();
            let run_time_ticks = (duration_secs * 10_000_000.0) as u64;
            let _bitrate = props.overall_bitrate().unwrap_or(0) as u64;

            let (title, artist_name, track_num, year, genres) = match tagged.primary_tag().or_else(|| tagged.first_tag()) {
                Some(tag) => {
                    let title = {
                        let t = read_tag_string(tag, lofty::tag::ItemKey::TrackTitle);
                        if t.is_empty() {
                            file.file_stem().unwrap_or_default().to_string_lossy().to_string()
                        } else {
                            t
                        }
                    };
                    let artist = {
                        let a = read_tag_string(tag, lofty::tag::ItemKey::TrackArtist);
                        if a.is_empty() { album_artist.clone() } else { a }
                    };
                    let num = tag.track().unwrap_or(1) as u64;
                    let y = tag.year().unwrap_or(0) as u64;
                    let g: Vec<String> = tag
                        .get_string(&lofty::tag::ItemKey::Genre)
                        .map(|s| vec![s.to_string()])
                        .unwrap_or_default();
                    (title, artist, num, y, g)
                }
                None => (
                    file.file_stem().unwrap_or_default().to_string_lossy().to_string(),
                    album_artist.clone(),
                    1u64,
                    0u64,
                    vec![],
                ),
            };

            let track_json = serde_json::json!({
                "Id": track_id,
                "Name": title,
                "Album": album_name,
                "AlbumId": album_key,
                "AlbumArtist": album_artist,
                "AlbumArtists": [{"Id": artist_id, "Name": album_artist}],
                "Artists": [artist_name],
                "IndexNumber": track_num,
                "ParentIndexNumber": 1,
                "ProductionYear": year,
                "RunTimeTicks": run_time_ticks,
                "Genres": genres,
                "HasLyrics": false,
                "download_status": "Downloaded",
                "file_path": path_str,
                "ServerId": "",
                "ParentId": album_key,
                "DateCreated": "",
                "MediaType": "",
                "PremiereDate": "",
                "BackdropImageTags": [],
                "ChannelId": null,
                "IsFolder": false,
                "MediaSources": [],
                "NormalizationGain": 0.0,
                "PlaylistItemId": "",
                "UserData": {},
                "disliked": false
            })
            .to_string();

            sqlx::query(
                r#"INSERT INTO tracks (id, album_id, artist_items, download_status, track)
                   VALUES (?, ?, ?, ?, ?)
                   ON CONFLICT(id) DO UPDATE SET
                       track = excluded.track,
                       download_status = excluded.download_status"#,
            )
            .bind(&track_id)
            .bind(&album_key)
            .bind("[]")
            .bind("Downloaded")
            .bind(&track_json)
            .execute(&mut *tx)
            .await?;

            sqlx::query(
                r#"INSERT INTO artist_membership (artist_id, track_id)
                   VALUES (?, ?)
                   ON CONFLICT DO NOTHING"#,
            )
            .bind(&artist_id)
            .bind(&track_id)
            .execute(&mut *tx)
            .await?;

            stats.inserted += 1;
        }
    }

    tx.commit().await?;
    log::info!(
        "Scan complete: {} files scanned, {} inserted/updated, {} errors",
        stats.scanned,
        stats.inserted,
        stats.errors
    );
    Ok(stats)
}
