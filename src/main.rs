// MIT License
// Copyright (c) 2026 Cedric Gegout

pub mod cli;
pub mod config;
pub mod digest;
pub mod error;
pub mod gemini;
pub mod gmail;
pub mod mattermost;
pub mod models;
pub mod telegram;
pub mod telegram_commands;
pub mod telegram_format;
pub mod system_status;

use chrono::Utc;
use clap::Parser;
use std::fs;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer};

use crate::cli::{AuthCommand, Cli, Commands, TestCommand};
use crate::config::Config;
use crate::error::AppError;
use crate::mattermost::{MattermostApi, MattermostClient};

/// Application entrypoint
#[tokio::main]
async fn main() {
    // Install the rustls crypto provider to support HTTPS requests via reqwest
    let _ = rustls::crypto::ring::default_provider().install_default();
    
    // Execute the main run logic and catch any propagated AppErrors
    if let Err(e) = run().await {
        tracing::error!("Application error: {}", e);
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

/// Core execution flow orchestrating CLI parsing, API clients, and the digest logic.
async fn run() -> Result<(), AppError> {
    // Attempt to load config early to get log level, if possible.
    // Otherwise fallback to "info".
    let log_level = Config::load()
        .map(|c| c.logging.level)
        .unwrap_or_else(|_| "info".to_string());

    // Setup logging to a file in the current directory
    let file_appender = tracing_appender::rolling::never(".", "mattermost-digest.out");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    let env_filter = EnvFilter::new(&log_level);
    
    // Setup standard output logging
    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stdout)
        .with_filter(EnvFilter::new(&log_level));
        
    // Combine file and stdout tracing subscribers
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_filter(env_filter);

    let _ = tracing_subscriber::registry()
        .with(stdout_layer)
        .with(file_layer)
        .try_init();

    // Parse the CLI arguments
    let cli = Cli::parse();
    let mut config = Config::load()?;

    // Route execution based on the chosen command
    match cli.command {
        Commands::Run { dry_run, lookback_hours, my_username, max_posts_per_channel } => {
            // Apply CLI overrides to configuration if provided
            if let Some(h) = lookback_hours {
                config.mattermost.lookback_hours = h;
                tracing::info!("Overriding lookback_hours via CLI to {}", h);
            }
            if let Some(u) = my_username {
                config.mattermost.my_username = u;
                tracing::info!("Overriding my_username via CLI to {}", config.mattermost.my_username);
            }
            if let Some(m) = max_posts_per_channel {
                config.output.max_posts_per_channel = m;
                tracing::info!("Overriding max_posts_per_channel via CLI to {}", m);
            }

            // Initialize Mattermost API client and fetch the latest messages
            let mm_client = MattermostClient::new(&config.mattermost)?;
            let now = Utc::now();
            let result = digest::generate_digest(&mm_client, &config, now).await?;
            
            let md_path = config::expand_tilde(&config.output.markdown_path);
            
            // Ensure the directory for the raw output file exists
            if let Some(parent) = md_path.parent() {
                fs::create_dir_all(parent)?;
            }
            
            let mut final_markdown = result.markdown.clone();
            
            // Generate a Gemini-powered executive summary if activity was found
            if result.has_messages {
                tracing::info!("Summarizing digest with Gemini...");
                match gemini::summarize_digest(&config, &result.markdown).await {
                    Ok(summary) => {
                        tracing::info!("Successfully obtained summary from Gemini.");
                        final_markdown = format!("# Executive Summary\n\n{}\n\n---\n\n{}", summary, result.markdown);
                    }
                    Err(e) => {
                        tracing::error!("Failed to summarize with Gemini: {}", e);
                    }
                }
            }
            
            // Write out the Markdown to disk
            fs::write(&md_path, &final_markdown)?;
            tracing::info!("Wrote markdown digest to {:?}", md_path);

            // Convert the Markdown text into styled HTML using pulldown-cmark
            tracing::info!("Converting markdown to HTML for email...");
            let parser = pulldown_cmark::Parser::new(&final_markdown);
            let mut html_output = String::new();
            pulldown_cmark::html::push_html(&mut html_output, parser);

            let styled_html = format!(
                "<html>\n<head>\n<style>\n\
                 body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Helvetica, Arial, sans-serif; line-height: 1.6; color: #333; max-width: 800px; margin: 0 auto; padding: 20px; }}\n\
                 h1 {{ color: #2D3139; border-bottom: 2px solid #E95420; padding-bottom: 8px; }}\n\
                 h2 {{ color: #E95420; margin-top: 24px; font-size: 1.3em; }}\n\
                 h3 {{ color: #555; margin-top: 16px; font-size: 1.1em; }}\n\
                 ul {{ padding-left: 20px; }}\n\
                 li {{ margin-bottom: 6px; }}\n\
                 strong {{ color: #2D3139; }}\n\
                 hr {{ border: 0; border-top: 1px solid #ccc; margin: 30px 0; }}\n\
                 </style>\n</head>\n<body>\n{}\n</body>\n</html>",
                html_output
            );
            
            // Email dispatching rules
            if dry_run {
                tracing::info!("Dry run enabled. Skipping email send.");
            } else if !result.has_messages {
                tracing::info!("No messages in digest. Still sending email to confirm empty window.");
                gmail::send_digest_email(&config.gmail, &styled_html).await?;
                tracing::info!("Email sent successfully.");
            } else {
                tracing::info!("Sending email...");
                gmail::send_digest_email(&config.gmail, &styled_html).await?;
                tracing::info!("Email sent successfully.");
            }
        }
        
        Commands::Auth { service } => match service {
            AuthCommand::Gmail => {
                tracing::info!("Starting Gmail authentication flow...");
                gmail::test_auth(&config.gmail).await?;
                tracing::info!("Gmail authentication successful!");
            }
        },
        
        Commands::Test { service } => match service {
            TestCommand::Mattermost => {
                tracing::info!("Testing Mattermost connection...");
                let mm_client = MattermostClient::new(&config.mattermost)?;
                let me = mm_client.get_me().await?;
                tracing::info!("Successfully connected as: {} ({})", me.username, me.email);
            }
            TestCommand::Gmail => {
                tracing::info!("Testing Gmail connection...");
                gmail::test_auth(&config.gmail).await?;
                tracing::info!("Successfully connected to Gmail!");
            }
            TestCommand::Gemini => {
                gemini::test_connection(&config).await?;
            }
        },
        
        Commands::PrintConfig => {
            // Clones the configuration structure and censors tokens/keys before logging it.
            let mut safe_config = config.clone();
            safe_config.mattermost.personal_token = "********".to_string();
            safe_config.gemini.api_key = "********".to_string();
            let toml_string = toml::to_string_pretty(&safe_config).unwrap();
            println!("{}", toml_string);
        }
        
        Commands::Bot => {
            crate::telegram::run_bot(config).await;
        }
    }

    Ok(())
}
