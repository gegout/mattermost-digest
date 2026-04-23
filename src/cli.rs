// MIT License
// Copyright (c) 2026 Cedric Gegout

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(author, version, about = "Mattermost Digest generator", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the digest process
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
    
    /// Authenticate or test services
    Auth {
        #[command(subcommand)]
        service: AuthCommand,
    },
    
    /// Test a specific service
    Test {
        #[command(subcommand)]
        service: TestCommand,
    },
    
    /// Print the current configuration
    PrintConfig,
}

#[derive(Subcommand, Debug)]
pub enum AuthCommand {
    /// Authenticate with Gmail
    Gmail,
}

#[derive(Subcommand, Debug)]
pub enum TestCommand {
    /// Test Mattermost connection
    Mattermost,
    /// Test Gmail connection
    Gmail,
    /// Test Gemini connection
    Gemini,
}
