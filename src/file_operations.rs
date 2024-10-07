use crate::errors::MyError;
use crate::{Channel, Conversation};
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};

#[cfg(feature = "zip")]
use tempfile::TempDir;

#[cfg(feature = "zip")]
use zip::read::ZipArchive;

#[cfg(feature = "zip")]
pub struct DataRoot {
    pub path: PathBuf,
    #[allow(dead_code)]
    temp_dir: Option<TempDir>,
}

#[cfg(not(feature = "zip"))]
pub struct DataRoot {
    pub path: PathBuf,
}

type Mappings = (Option<HashMap<String, String>>, Option<HashMap<String, String>>);

pub fn prepare_data_root(input_path: &Path) -> Result<DataRoot, MyError> {
    #[cfg(feature = "zip")]
    {
        if input_path.is_file() {
            // Handle ZIP file
            let file = File::open(input_path)?;
            let mut archive = ZipArchive::new(file)?;
            let temp_dir = TempDir::new()?;
            archive.extract(temp_dir.path())?;
            Ok(DataRoot {
                path: temp_dir.path().to_path_buf(),
                temp_dir: Some(temp_dir),
            })
        } else if input_path.is_dir() {
            Ok(DataRoot {
                path: input_path.to_path_buf(),
                temp_dir: None,
            })
        } else {
            Err(MyError::InvalidInputPath(input_path.display().to_string()))
        }
    }

    #[cfg(not(feature = "zip"))]
    {
        if input_path.is_dir() {
            Ok(DataRoot {
                path: input_path.to_path_buf(),
            })
        } else {
            Err(MyError::InvalidInputPath(input_path.display().to_string()))
        }
    }
}

pub fn load_mappings(
    data_root: &DataRoot,
) -> Result<Mappings, MyError> {
    let messages_folder = data_root.path.join("messages");
    let servers_folder = data_root.path.join("servers");

    let channel_mapping = load_mapping(&messages_folder.join("index.json"))?;
    let guild_mapping = load_mapping(&servers_folder.join("index.json"))?;

    Ok((channel_mapping, guild_mapping))
}

fn load_mapping(path: &Path) -> Result<Option<HashMap<String, String>>, MyError> {
    if path.exists() {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mapping = serde_json::from_reader(reader)?;
        Ok(Some(mapping))
    } else {
        Ok(None)
    }
}

pub fn process_conversations(
    data_root: &DataRoot,
    channel_mapping: &Option<HashMap<String, String>>,
    guild_mapping: &Option<HashMap<String, String>>,
) -> Result<Vec<Conversation>, MyError> {
    let messages_folder = data_root.path.join("messages");
    let entries = fs::read_dir(messages_folder)?;

    let progress = ProgressBar::new_spinner();
    progress.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .map_err(|e| MyError::ProgressBar(e.to_string()))?,
    );
    progress.enable_steady_tick(std::time::Duration::from_millis(100));
    progress.set_message("Processing conversations...");

    let mut conversations = Vec::new();
    let mut guilds = HashMap::new();

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            let channel_id = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| MyError::InvalidInputPath(format!("Invalid channel ID in path: {}", path.display())))?
                .to_string();

            let messages_file = path.join("messages.json");
            let channel_info_file = path.join("channel.json");

            if messages_file.exists() && channel_info_file.exists() {
                let channel_info: Value = read_json(&channel_info_file)?;
                let messages: Vec<Value> = read_json(&messages_file)?;
                let channel_message_count = messages.len();

                if let Some(guild_info) = channel_info.get("guild") {
                    let guild_id = guild_info
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Unknown");
                    let guild_name = guild_mapping
                        .as_ref()
                        .and_then(|gm| gm.get(guild_id))
                        .cloned()
                        .unwrap_or_else(|| format!("Guild {}", guild_id));
                    let channel_name = channel_info
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&channel_id)
                        .to_string();

                    let guild = guilds
                        .entry(guild_id.to_string())
                        .or_insert_with(|| Conversation::Guild {
                            name: guild_name.clone(),
                            message_count: 0,
                            channels: Vec::new(),
                        });

                    if let Conversation::Guild {
                        message_count,
                        channels,
                        ..
                    } = guild
                    {
                        *message_count += channel_message_count;
                        channels.push(Channel {
                            name: channel_name,
                            message_count: channel_message_count,
                        });
                    }
                } else {
                    // DM or GC
                    let stripped_channel_id = channel_id.trim_start_matches('c');
                    let conversation_name = channel_mapping
                        .as_ref()
                        .and_then(|cm| cm.get(stripped_channel_id))
                        .cloned()
                        .unwrap_or_else(|| format!("Conversation {}", channel_id));

                    conversations.push(Conversation::DmOrGc {
                        name: conversation_name,
                        message_count: channel_message_count,
                    });
                }
            }
        }
    }

    progress.finish_and_clear();

    // Combine guilds into conversations
    conversations.extend(guilds.into_values());

    Ok(conversations)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, MyError> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let data = serde_json::from_reader(reader)?;
    Ok(data)
}
