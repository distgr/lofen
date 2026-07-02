use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::tag::Accessor;
use sqlx::{Pool, Sqlite};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
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
}

struct TrackInfo {
    path: String,
    track_id: String,
    album_key: String,
    album_artist: String,
    album_name: String,
    artist_id: String,
    title: String,
    artist_name: String,
    track_num: u64,
    year: u64,
    genres: Vec<String>,
    run_time_ticks: u64,
    has_lyrics: bool,
}

fn has_embedded_lyrics(tag: Option<&lofty::tag::Tag>) -> bool {
    tag.and_then(|t| t.get_string(&lofty::tag::ItemKey::Lyrics))
    .map(|s| !s.trim().is_empty())
    .unwrap_or(false)
}

pub async fn scan_paths(
    pool: &Arc<Pool<Sqlite>>,
    paths: &[String],
) -> Result<ScanStats, Box<dyn std::error::Error + Send + Sync>> {
    let paths_owned: Vec<String> = paths.to_vec();
    let covers_dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("lofen")
        .join("covers");

    log::info!("Scanner: starting file collection for {} paths", paths_owned.len());

    // Run all blocking I/O (walkdir + lofty + cover extraction) on a thread-pool thread
    let track_infos = tokio::task::spawn_blocking(move || {
        collect_track_infos(&paths_owned, &covers_dir)
    })
    .await??;

    log::info!("Scanner: collected {} tracks, starting DB writes", track_infos.len());

    let scanned = track_infos.len();
    let errors = 0usize;

    // Group by artist for upserts
    let mut artists: HashMap<String, (String, String)> = HashMap::new(); // artist_id -> (name, json)
    let mut albums: HashMap<String, (String, String, String)> = HashMap::new(); // album_key -> (album_key, artist_id, json)

    for t in &track_infos {
        artists.entry(t.artist_id.clone()).or_insert_with(|| {
            let json = serde_json::json!({
                "Id": t.artist_id,
                "Name": t.album_artist,
                "RunTimeTicks": 0,
                "Type": "",
                "UserData": {},
                "DateCreated": ""
            })
            .to_string();
            (t.album_artist.clone(), json)
        });

        albums.entry(t.album_key.clone()).or_insert_with(|| {
            let json = serde_json::json!({
                "Id": t.album_key,
                "Name": t.album_name,
                "AlbumArtists": [{"Id": t.artist_id, "Name": t.album_artist}],
                "UserData": {},
                "DateCreated": "",
                "ParentId": "",
                "RunTimeTicks": 0,
                "ProductionYear": 0,
                "PremiereDate": ""
            })
            .to_string();
            (t.album_key.clone(), t.artist_id.clone(), json)
        });
    }

    let mut tx = pool.begin().await?;

    for (artist_id, (_, artist_json)) in &artists {
        sqlx::query(
            "INSERT INTO artists (id, artist) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET artist = excluded.artist",
        )
        .bind(artist_id)
        .bind(artist_json)
        .execute(&mut *tx)
        .await?;
    }

    for (album_key, (_, _, album_json)) in &albums {
        sqlx::query(
            r#"INSERT INTO albums (id, album) VALUES (?, ?)
               ON CONFLICT(id) DO UPDATE SET album = excluded.album"#,
        )
        .bind(album_key)
        .bind(album_json)
        .execute(&mut *tx)
        .await?;
    }

    let mut inserted = 0usize;
    for t in &track_infos {
        let track_json = serde_json::json!({
            "Id": t.track_id,
            "Name": t.title,
            "Album": t.album_name,
            "AlbumId": t.album_key,
            "AlbumArtist": t.album_artist,
            "AlbumArtists": [{"Id": t.artist_id, "Name": t.album_artist}],
            "Artists": [t.artist_name],
            "IndexNumber": t.track_num,
            "ParentIndexNumber": 1,
            "ProductionYear": t.year,
            "RunTimeTicks": t.run_time_ticks,
            "Genres": t.genres,
            "HasLyrics": t.has_lyrics,
            "download_status": "Downloaded",
            "file_path": t.path,
            "ServerId": "",
            "ParentId": t.album_key,
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
            r#"INSERT INTO tracks (id, album_id, download_status, track)
               VALUES (?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   track = excluded.track,
                   download_status = excluded.download_status"#,
        )
        .bind(&t.track_id)
        .bind(&t.album_key)
        .bind("Downloaded")
        .bind(&track_json)
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            r#"INSERT INTO artist_membership (artist_id, track_id)
               VALUES (?, ?)
               ON CONFLICT DO NOTHING"#,
        )
        .bind(&t.artist_id)
        .bind(&t.track_id)
        .execute(&mut *tx)
        .await?;

        inserted += 1;
    }

    tx.commit().await?;

    log::info!(
        "Scan complete: {} files scanned, {} inserted/updated, {} errors",
        scanned,
        inserted,
        errors
    );

    Ok(ScanStats { scanned, inserted })
}

fn extract_cover(
    tagged: &lofty::file::TaggedFile,
    album_key: &str,
    covers_dir: &Path,
    seen: &mut HashSet<String>,
) {
    if seen.contains(album_key) {
        return;
    }
    seen.insert(album_key.to_string());

    let tag = match tagged.primary_tag().or_else(|| tagged.first_tag()) {
        Some(t) => t,
        None => return,
    };

    let pictures = tag.pictures();
    if pictures.is_empty() {
        return;
    }

    let pic = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first());

    if let Some(pic) = pic {
        let ext = pic
            .mime_type()
            .and_then(|m| m.ext())
            .unwrap_or("jpg");
        let dest = covers_dir.join(format!("{}.{}", album_key, ext));
        if !dest.exists() {
            if let Err(e) = std::fs::write(&dest, pic.data()) {
                log::warn!("Scanner: failed to write cover art for {}: {}", album_key, e);
            } else {
                log::info!("Scanner: saved cover art → {}", dest.display());
            }
        }
    }
}

fn collect_track_infos(
    paths: &[String],
    covers_dir: &Path,
) -> Result<Vec<TrackInfo>, Box<dyn std::error::Error + Send + Sync>> {
    let _ = std::fs::create_dir_all(covers_dir);
    let mut seen_covers: HashSet<String> = HashSet::new();
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for root in paths {
        log::info!("Scanner: walking {}", root);
        for entry in WalkDir::new(root).follow_links(true).into_iter().filter_map(|e| e.ok()) {
            let path = entry.path().to_path_buf();
            if path.is_file() && is_audio(&path) {
                files.push(path);
            }
        }
    }
    log::info!("Scanner: found {} audio files", files.len());

    let mut infos = Vec::with_capacity(files.len());

    for (i, file) in files.iter().enumerate() {
        log::info!("Scanner: reading tags [{}/{}] {}", i + 1, files.len(), file.display());
        let path_str = file.to_string_lossy().to_string();
        let track_id = path_id(&path_str);

        let tagged = match lofty::read_from_path(file) {
            Ok(t) => { log::info!("Scanner: OK {}", file.display()); t }
            Err(e) => {
                log::warn!("Scanner: FAILED {} - {}", file.display(), e);
                // Still add the file with unknown metadata
                let album_artist = "Unknown Artist".to_string();
                let album_name = "Unknown Album".to_string();
                let album_key = path_id(&format!("{}{}", album_artist, album_name));
                let artist_id = path_id(&album_artist.to_ascii_lowercase());
                let title = file.file_stem().unwrap_or_default().to_string_lossy().to_string();
                infos.push(TrackInfo {
                    path: path_str,
                    track_id,
                    album_key,
                    album_artist,
                    album_name,
                    artist_id,
                    title,
                    artist_name: "Unknown Artist".to_string(),
                    track_num: 1,
                    year: 0,
                    genres: vec![],
                    run_time_ticks: 0,
                    has_lyrics: false,
                });
                continue;
            }
        };

        let props = tagged.properties();
        let run_time_ticks = (props.duration().as_secs_f64() * 10_000_000.0) as u64;

        let tag = tagged.primary_tag().or_else(|| tagged.first_tag());

        let album_artist = tag
            .map(|t| {
                let aa = read_tag_string(t, lofty::tag::ItemKey::AlbumArtist);
                if aa.is_empty() {
                    read_tag_string(t, lofty::tag::ItemKey::TrackArtist)
                } else {
                    aa
                }
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Unknown Artist".to_string());

        let album_name = tag
            .map(|t| read_tag_string(t, lofty::tag::ItemKey::AlbumTitle))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Unknown Album".to_string());

        let title = tag
            .map(|t| read_tag_string(t, lofty::tag::ItemKey::TrackTitle))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| file.file_stem().unwrap_or_default().to_string_lossy().to_string());

        let artist_name = tag
            .map(|t| read_tag_string(t, lofty::tag::ItemKey::TrackArtist))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| album_artist.clone());

        let track_num = tag.and_then(|t| t.track()).unwrap_or(1) as u64;
        let year = tag.and_then(|t| t.year()).unwrap_or(0) as u64;
        let genres = tag
            .and_then(|t| t.get_string(&lofty::tag::ItemKey::Genre))
            .map(|s| vec![s.to_string()])
            .unwrap_or_default();
        let has_lyrics = has_embedded_lyrics(tag);

        let album_key = path_id(&format!("{}{}", album_artist, album_name));
        let artist_id = path_id(&album_artist.to_ascii_lowercase());

        extract_cover(&tagged, &album_key, covers_dir, &mut seen_covers);

        infos.push(TrackInfo {
            path: path_str,
            track_id,
            album_key,
            album_artist,
            album_name,
            artist_id,
            title,
            artist_name,
            track_num,
            year,
            genres,
            run_time_ticks,
            has_lyrics,
        });
    }

    Ok(infos)
}
