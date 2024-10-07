use clap::{Parser, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};
use tempfile::tempdir;
use zip::read::ZipArchive;

/// Discord Message Counter
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// Path to the Discord data package (ZIP file or extracted folder)
    input_path: PathBuf,

    /// Limit the number of conversations displayed
    #[arg(short, long)]
    limit: Option<usize>,

    /// Filter by conversation type (dm, guild)
    #[arg(short, long, value_enum, value_name = "TYPE")]
    conversation_type: Option<ConversationType>,

    /// Minimum message count to display
    #[arg(short, long, default_value_t = 1)]
    min_messages: usize,
}

#[derive(ValueEnum, Clone, Debug)]
enum ConversationType {
    Dm,
    Guild,
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
    let cli = Cli::parse();

    // Prepare data root
    let data_root = match prepare_data_root(&cli.input_path) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("Error: {}", e);
            return;
        }
    };

    // Load mappings
    let (channel_mapping, guild_mapping) = match load_mappings(&data_root) {
        Ok(mappings) => mappings,
        Err(e) => {
            eprintln!("Error loading mappings: {}", e);
            return;
        }
    };

    // Process conversations
    let conversations = match process_conversations(&data_root, &channel_mapping, &guild_mapping) {
        Ok(convs) => convs,
        Err(e) => {
            eprintln!("Error processing conversations: {}", e);
            return;
        }
    };

    // Filter and sort conversations
    let filtered_conversations =
        filter_and_sort_conversations(conversations, &cli.conversation_type, cli.min_messages, cli.limit);

    // Print conversations
    print_conversations(filtered_conversations);
}

fn prepare_data_root(input_path: &Path) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if input_path.is_file() {
        // Try to open it as a ZIP file
        let file = File::open(input_path)?;
        let mut archive = ZipArchive::new(file)?;
        let temp_dir = tempdir()?;
        archive.extract(&temp_dir)?;
        Ok(temp_dir.into_path())
    } else if input_path.is_dir() {
        Ok(input_path.to_path_buf())
    } else {
        Err(format!("Invalid input path: {}", input_path.display()).into())
    }
}

fn load_mappings(
    data_root: &Path,
) -> Result<(Option<HashMap<String, String>>, Option<HashMap<String, String>>), Box<dyn std::error::Error>> {
    let messages_folder = data_root.join("messages");
    let servers_folder = data_root.join("servers");

    let channel_mapping = load_mapping(&messages_folder.join("index.json"))?;
    let guild_mapping = load_mapping(&servers_folder.join("index.json"))?;

    Ok((channel_mapping, guild_mapping))
}

fn load_mapping(path: &Path) -> Result<Option<HashMap<String, String>>, Box<dyn std::error::Error>> {
    if path.exists() {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let mapping = serde_json::from_reader(reader)?;
        Ok(Some(mapping))
    } else {
        Ok(None)
    }
}

fn process_conversations(
    data_root: &Path,
    channel_mapping: &Option<HashMap<String, String>>,
    guild_mapping: &Option<HashMap<String, String>>,
) -> Result<Vec<Conversation>, Box<dyn std::error::Error>> {
    let messages_folder = data_root.join("messages");
    let entries = fs::read_dir(&messages_folder)?;

    let progress = ProgressBar::new_spinner();
    progress.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner} {msg}")
            .expect("Failed to set progress bar template"),
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
                .ok_or("Failed to get channel ID")?
                .to_string_lossy()
                .into_owned();

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

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, Box<dyn std::error::Error>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let data = serde_json::from_reader(reader)?;
    Ok(data)
}

fn filter_and_sort_conversations(
    conversations: Vec<Conversation>,
    conversation_type: &Option<ConversationType>,
    min_messages: usize,
    limit: Option<usize>,
) -> Vec<Conversation> {
    let mut filtered: Vec<_> = conversations
        .into_iter()
        .filter(|conv| {
            if conv.message_count() < min_messages {
                return false;
            }
            if let Some(ref ctype) = conversation_type {
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
    filtered.sort_by(|a, b| b.message_count().cmp(&a.message_count()));

    // Apply limit if specified
    if let Some(limit) = limit {
        filtered.truncate(limit);
    }

    filtered
}

fn print_conversations(conversations: Vec<Conversation>) {
    for conversation in conversations {
        conversation.print_tree();
    }
}
