use clap::{Parser, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::PathBuf;

mod errors;
mod file_operations;

use errors::MyError;
use file_operations::{load_mappings, prepare_data_root, process_conversations};

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
            Self::DmOrGc { message_count, .. } => *message_count,
            Self::Guild { message_count, .. } => *message_count,
        }
    }

    fn print_tree(&self) {
        match self {
            Self::DmOrGc { name, message_count } => {
                println!("{} [{} messages]", name, message_count);
            }
            Self::Guild {
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

fn main() -> Result<(), MyError> {
    let cli = Cli::parse();

    // Prepare data root
    let data_root = prepare_data_root(&cli.input_path)?;

    // Load mappings
    let (channel_mapping, guild_mapping) = load_mappings(&data_root)?;

    // Process conversations
    let conversations = process_conversations(
        &data_root,
        &channel_mapping,
        &guild_mapping,
    )?;

    // Filter and sort conversations
    let filtered_conversations = filter_and_sort_conversations(
        conversations,
        &cli.conversation_type,
        cli.min_messages,
        cli.limit,
    );

    // Print conversations
    print_conversations(filtered_conversations);

    Ok(())
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
                matches!(
                    (ctype, conv),
                    (ConversationType::Dm, Conversation::DmOrGc { .. })
                        | (ConversationType::Guild, Conversation::Guild { .. })
                )
            } else {
                true
            }
        })
        .collect();

    // Sort conversations by message count in descending order
    filtered.sort_unstable_by(|a, b| b.message_count().cmp(&a.message_count()));

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
