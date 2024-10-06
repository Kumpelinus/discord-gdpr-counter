use argh::FromArgs;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use zip::read::ZipArchive;

/// Discord Message Counter
#[derive(FromArgs)]
struct Cli {
    /// path to the Discord data package (ZIP file or extracted folder)
    #[argh(positional)]
    input_path: PathBuf,

    /// limit the number of conversations displayed
    #[argh(option, short = 'l')]
    limit: Option<usize>,

    /// filter by conversation type (dm, guild)
    #[argh(option, short = 't')]
    conversation_type: Option<ConversationType>,

    /// minimum message count to display
    #[argh(option, short = 'm', default = "1")]
    min_messages: usize,
}

#[derive(Debug, Clone)]
enum ConversationType {
    Dm,
    Guild,
}

impl std::str::FromStr for ConversationType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "dm" => Ok(ConversationType::Dm),
            "guild" => Ok(ConversationType::Guild),
            _ => Err(format!("Invalid conversation type: {}", s)),
        }
    }
}

#[derive(Debug)]
enum Conversation {
    DmOrGc {
        name: String,
        message_count: usize,
    },
    Guild {
        name: String,
        message_count: usize,
        channels: Vec<Channel>,
    },
}

#[derive(Debug, Clone)]
struct Channel {
    name: String,
    message_count: usize,
}

impl Conversation {
    fn message_count(&self) -> usize {
        match self {
            Conversation::DmOrGc { message_count, .. } => *message_count,
            Conversation::Guild { message_count, .. } => *message_count,
        }
    }

    fn print_tree(&self) {
        match self {
            Conversation::DmOrGc { name, message_count } => {
                println!("{} [{} messages]", name, message_count);
            }
            Conversation::Guild {
                name,
                message_count,
                channels,
            } => {
                println!("{} [{} messages]", name, message_count);
                let mut sorted_channels = channels.clone();
                sorted_channels.sort_unstable_by(|a, b| b.message_count.cmp(&a.message_count));
                for (i, channel) in sorted_channels.iter().enumerate() {
                    let connector = if i == sorted_channels.len() - 1 { "└──" } else { "├──" };
                    println!(
                        "    {} {} [{} messages]",
                        connector, channel.name, channel.message_count
                    );
                }
                println!();
            }
        }
    }
}

fn main() {
    let cli: Cli = argh::from_env();

    // Determine the data path
    let data_root = match prepare_data_root(&cli.input_path) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("Failed to prepare data root: {}", e);
            return;
        }
    };

    // Load the channel and guild mappings
    let messages_folder = data_root.join("messages");
    let servers_folder = data_root.join("servers");

    let channel_mapping = load_mapping(&messages_folder.join("index.json"));
    let guild_mapping = load_mapping(&servers_folder.join("index.json"));

    let mut conversations = Vec::new();
    let mut guilds = HashMap::new();

    // Create a progress bar
    let progress = ProgressBar::new_spinner();
    progress.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .expect("Failed to set progress bar template"),
    );
    progress.enable_steady_tick(std::time::Duration::from_millis(100));
    progress.set_message("Processing conversations...");

    for entry in match fs::read_dir(&messages_folder) {
        Ok(entries) => entries,
        Err(e) => {
            eprintln!("Failed to read messages directory: {}", e);
            return;
        }
    } {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("Failed to read entry in messages directory: {}", e);
                continue;
            }
        };
        let path = entry.path();

        if path.is_dir() {
            let channel_id = match path.file_name() {
                Some(name) => name.to_string_lossy().into_owned(),
                None => {
                    eprintln!("Failed to get channel ID from path: {:?}", path);
                    continue;
                }
            };

            let messages_file = path.join("messages.json");
            let channel_info_file = path.join("channel.json");

            if messages_file.exists() && channel_info_file.exists() {
                let channel_info: serde_json::Value = match read_json(&channel_info_file) {
                    Ok(info) => info,
                    Err(e) => {
                        eprintln!("Failed to read channel info: {}", e);
                        continue;
                    }
                };
                let messages: Vec<serde_json::Value> = match read_json(&messages_file) {
                    Ok(msgs) => msgs,
                    Err(e) => {
                        eprintln!("Failed to read messages: {}", e);
                        continue;
                    }
                };
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
                        message_count: guild_message_count,
                        channels,
                        ..
                    } = guild
                    {
                        *guild_message_count += channel_message_count;
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

    // Filter conversations based on user input
    let mut filtered_conversations: Vec<_> = conversations
        .into_iter()
        .chain(guilds.into_values())
        .filter(|conv| {
            if conv.message_count() < cli.min_messages {
                return false;
            }
            if let Some(ref ctype) = cli.conversation_type {
                match (ctype, conv) {
                    (ConversationType::Dm, Conversation::DmOrGc { .. }) => true,
                    (ConversationType::Guild, Conversation::Guild { .. }) => true,
                    _ => false,
                }
            } else {
                true
            }
        })
        .collect();

    // Sort conversations by message count in descending order
    filtered_conversations.sort_by(|a, b| b.message_count().cmp(&a.message_count()));

    // Apply limit if specified
    if let Some(limit) = cli.limit {
        filtered_conversations.truncate(limit);
    }

    // Print the conversations
    for conversation in filtered_conversations {
        conversation.print_tree();
    }
}

fn prepare_data_root(input_path: &PathBuf) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if input_path.is_file() {
        // Try to open it as a ZIP file
        let file = File::open(input_path)?;
        let mut archive = ZipArchive::new(file)?;
        let temp_dir = tempdir()?;
        archive.extract(&temp_dir)?;
        Ok(temp_dir.into_path())
    } else if input_path.is_dir() {
        Ok(input_path.clone())
    } else {
        Err(format!(
            "Input path must be a file or directory: {}",
            input_path.display()
        )
        .into())
    }
}

fn load_mapping(path: &Path) -> Option<HashMap<String, String>> {
    if path.exists() {
        let file = File::open(path).ok()?;
        let reader = BufReader::new(file);
        serde_json::from_reader(reader).ok()
    } else {
        None
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, serde_json::Error> {
    let file = File::open(path).map_err(serde_json::Error::io)?;
    let reader = BufReader::new(file);
    serde_json::from_reader(reader)
}
