use crate::themes::dialoguer::DialogTheme;
use dirs::{config_dir, data_dir};
use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy)]
pub enum LyricsVisibility {
    Always,
    Auto,
    Never,
}
impl LyricsVisibility {
    pub fn from_config(val: &str) -> Self {
        match val {
            "auto" => Self::Auto,
            "never" => Self::Never,
            _ => Self::Always,
        }
    }
}

/// This makes sure all dirs are created before we do anything.
pub fn prepare_directories() -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = data_dir().expect(" ! Failed getting data directory");
    let config_dir = config_dir().expect(" ! Failed getting config directory");

    let j_data_dir = data_dir.join("lofen");
    let j_config_dir = config_dir.join("lofen");

    std::fs::create_dir_all(&j_data_dir)?;
    std::fs::create_dir_all(&j_config_dir)?;

    std::fs::create_dir_all(j_data_dir.join("log"))?;
    std::fs::create_dir_all(j_data_dir.join("covers"))?;
    std::fs::create_dir_all(j_data_dir.join("states"))?;
    std::fs::create_dir_all(j_data_dir.join("preferences"))?;
    std::fs::create_dir_all(j_data_dir.join("downloads"))?;
    std::fs::create_dir_all(j_data_dir.join("databases"))?;
    std::fs::create_dir_all(j_data_dir.join("mpv-scripts"))?;

    Ok(())
}

pub fn get_config() -> Result<(PathBuf, serde_yaml::Value), Box<dyn std::error::Error>> {
    let config_dir = match config_dir() {
        Some(dir) => dir,
        None => return Err("Could not find config directory".into()),
    };

    let config_file: PathBuf = config_dir.join("lofen").join("config.yaml").into();

    if !config_file.exists() {
        return Ok((config_file, serde_yaml::Value::Mapping(Default::default())));
    }

    let f = std::fs::File::open(&config_file)?;
    let d = serde_yaml::from_reader(f)?;

    Ok((config_file, d))
}

/// Creates a minimal config file if none exists.
pub fn initialize_config() {
    let config_dir = match config_dir() {
        Some(dir) => dir,
        None => {
            println!(" ! Could not find config directory");
            std::process::exit(1);
        }
    };

    let config_file = config_dir.join("lofen").join("config.yaml");

    if config_file.exists() {
        println!(" - Config loaded: {}", config_file.display());
        return;
    }

    let default_config = "music_paths: []\n";

    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&config_file)
        .expect(" ! Could not create config file");
    file.write_all(default_config.as_bytes()).expect(" ! Could not write default config");

    println!(" - Created config file at: {}", config_file.display());
    println!(" - Add your music paths in the Settings tab (press 4).");
}

pub fn get_music_paths(config: &serde_yaml::Value) -> Vec<String> {
    config["music_paths"]
        .as_sequence()
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

pub fn save_music_paths(paths: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let config_dir = config_dir().ok_or("no config dir")?;
    let config_file = config_dir.join("lofen").join("config.yaml");

    let (_, mut config) = get_config()?;

    let paths_yaml: serde_yaml::Value = serde_yaml::Value::Sequence(
        paths.iter().map(|p| serde_yaml::Value::String(p.clone())).collect(),
    );

    if let serde_yaml::Value::Mapping(ref mut map) = config {
        map.insert(
            serde_yaml::Value::String("music_paths".to_string()),
            paths_yaml,
        );
    } else {
        let mut map = serde_yaml::Mapping::new();
        map.insert(
            serde_yaml::Value::String("music_paths".to_string()),
            paths_yaml,
        );
        config = serde_yaml::Value::Mapping(map);
    }

    let yaml_str = serde_yaml::to_string(&config)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&config_file)?;
    file.write_all(yaml_str.as_bytes())?;
    Ok(())
}

pub fn select_music_folder_interactive() -> Option<String> {
    let input = dialoguer::Input::<String>::with_theme(&DialogTheme::default())
        .with_prompt("Music folder path")
        .interact_text()
        .ok()?;
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}
