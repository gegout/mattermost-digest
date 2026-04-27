// MIT License
// Copyright (c) 2026 Cedric Gegout

use clap::{Parser, Subcommand};

/// The main command line interface structure for the application.
/// It uses `clap` to parse arguments and subcommands from the user.
#[derive(Parser, Debug)]
#[command(author, version, about = "Mattermost Digest generator", long_about = None)]
pub struct Cli {
    /// The specific subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,
}

/// Available subcommands for the CLI application.
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the digest process to fetch messages and summarize them.
    Run {
        /// Generate the markdown file but do not send the email
        #[arg(long, help = "Do not send email, only generate the digest file")]
        dry_run: bool,

        /// Number of hours to look back for messages (overrides config)
        #[arg(long, help = "Number of hours to look back for messages (overrides config)")]
        lookback_hours: Option<u32>,

        /// Your Mattermost username (overrides config)
        #[arg(long, help = "Your Mattermost username (overrides config)")]
        my_username: Option<String>,

        /// Maximum number of posts to include per channel (overrides config)
        #[arg(long, help = "Maximum number of posts to include per channel (overrides config)")]
        max_posts_per_channel: Option<u32>,
    },
    
    /// Authenticate or test services (e.g., initial OAuth flow)
    Auth {
        /// The specific service to authenticate with.
        #[command(subcommand)]
        service: AuthCommand,
    },
    
    /// Test a specific service's connection without running a full digest.
    Test {
        /// The specific service to test.
        #[command(subcommand)]
        service: TestCommand,
    },
    
    /// Print the current configuration (with sensitive fields redacted).
    PrintConfig,

    /// Start the interactive Telegram bot mode.
    Bot,
}

/// Services that support explicit authentication commands.
#[derive(Subcommand, Debug)]
pub enum AuthCommand {
    /// Authenticate with Gmail (triggers OAuth browser flow if needed).
    Gmail,
}

/// Services that support connectivity testing.
#[derive(Subcommand, Debug)]
pub enum TestCommand {
    /// Test Mattermost connection by fetching the current user profile.
    Mattermost,
    /// Test Gmail connection by fetching the current user profile.
    Gmail,
    /// Test Gemini API connection by requesting a simple summarization.
    Gemini,
}
