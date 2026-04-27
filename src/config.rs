// MIT License
// Copyright (c) 2026 Cedric Gegout

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::fs;
use crate::error::AppError;

/// The root configuration structure mapping to the `config.toml` file.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    /// Configuration for Mattermost connectivity.
    pub mattermost: MattermostConfig,
    /// Configuration for Gmail SMTP/API connectivity.
    pub gmail: GmailConfig,
    /// Configuration for Gemini API connectivity.
    pub gemini: GeminiConfig,
    /// Configuration governing output file formatting and constraints.
    pub output: OutputConfig,
    /// Configuration for application logging levels.
    pub logging: LoggingConfig,
}

/// Settings specific to the Mattermost API.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MattermostConfig {
    /// The base URL of the Mattermost server.
    #[serde(default = "default_base_url")]
    pub base_url: String,
    /// The personal access token to authenticate against Mattermost.
    pub personal_token: String,
    /// How many hours back the digest should search for messages.
    #[serde(default = "default_lookback_hours")]
    pub lookback_hours: u32,
    /// Timeout for Mattermost HTTP requests.
    #[serde(default = "default_request_timeout_seconds")]
    pub request_timeout_seconds: u64,
    /// Pagination size for API requests.
    #[serde(default = "default_per_page")]
    pub per_page: u32,
    /// The Mattermost username of the person requesting the digest.
    #[serde(default = "default_my_username")]
    pub my_username: String,
}

/// Settings specific to the Gmail API integration.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GmailConfig {
    /// Path to the Google OAuth client secret JSON file.
    pub client_secret_path: String,
    /// Path to cache the authorized OAuth token.
    pub token_cache_path: String,
    /// The email address to send the digest *from*.
    pub from_email: String,
    /// The email address to send the digest *to*.
    pub to_email: String,
    /// The subject line of the digest email.
    #[serde(default = "default_email_subject")]
    pub email_subject: String,
}

/// Settings for interacting with the Google Gemini API.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GeminiConfig {
    /// The API key for Gemini.
    pub api_key: String,
    /// The model string to use for summarization (e.g., gemini-2.5-flash).
    #[serde(default = "default_gemini_model")]
    pub model: String,
    /// The fallback model string to use if the primary model fails.
    #[serde(default = "default_gemini_fallback_model")]
    pub fallback_model: String,
    /// Maximum number of retries for API calls.
    #[serde(default = "default_gemini_max_retries")]
    pub max_retries: u32,
    /// Base delay in seconds for exponential backoff during retries.
    #[serde(default = "default_gemini_retry_delay_base_seconds")]
    pub retry_delay_base_seconds: u32,
}

/// Settings controlling the markdown output generation.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OutputConfig {
    /// Where the raw markdown file will be saved.
    pub markdown_path: String,
    /// Whether to list channels that had no new activity in the timeframe.
    #[serde(default = "default_false")]
    pub include_empty_channels: bool,
    /// The maximum number of posts to fetch/display per channel.
    #[serde(default = "default_max_posts")]
    pub max_posts_per_channel: u32,
}

/// Settings controlling application logging behavior.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoggingConfig {
    /// The filter level (e.g., 'info', 'debug', 'warn').
    #[serde(default = "default_log_level")]
    pub level: String,
}

// Default providers for serde deserialization
fn default_base_url() -> String { "https://chat.canonical.com".to_string() }
fn default_my_username() -> String { "cgegout".to_string() }
fn default_lookback_hours() -> u32 { 24 }
fn default_request_timeout_seconds() -> u64 { 30 }
fn default_per_page() -> u32 { 200 }
fn default_gemini_model() -> String { "gemini-2.5-flash".to_string() }
fn default_gemini_fallback_model() -> String { "gemini-1.5-flash".to_string() }
fn default_gemini_max_retries() -> u32 { 3 }
fn default_gemini_retry_delay_base_seconds() -> u32 { 2 }
fn default_email_subject() -> String { "Mattermost Digest".to_string() }
fn default_false() -> bool { false }
fn default_max_posts() -> u32 { 500 }
fn default_log_level() -> String { "info".to_string() }

impl Config {
    /// Loads the configuration from the standardized config path.
    /// Returns an error if the file is missing or cannot be parsed.
    pub fn load() -> Result<Self, AppError> {
        let config_path = get_config_path();
        if !config_path.exists() {
            return Err(AppError::Config(format!(
                "Config file not found at {:?}. Please create it.",
                config_path
            )));
        }

        let contents = fs::read_to_string(&config_path)
            .map_err(|e| AppError::Config(format!("Failed to read config file: {}", e)))?;

        let config: Config = toml::from_str(&contents)
            .map_err(|e| AppError::Config(format!("Failed to parse config TOML: {}", e)))?;

        Ok(config)
    }
}

/// Returns the base directory for storing application configuration.
pub fn get_config_dir() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"));
    path.push("mattermost-digest");
    path
}

/// Returns the base directory for storing state files like logs.
pub fn get_state_dir() -> PathBuf {
    let mut path = dirs::state_dir().unwrap_or_else(|| {
        let mut p = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
        p.push(".local");
        p.push("state");
        p
    });
    path.push("mattermost-digest");
    path
}

/// Returns the full path to `config.toml`.
pub fn get_config_path() -> PathBuf {
    get_config_dir().join("config.toml")
}

/// Expands a literal `~/` at the beginning of a path string into the actual home directory.
pub fn expand_tilde(path_str: &str) -> PathBuf {
    if path_str.starts_with("~/") {
        if let Some(mut home) = dirs::home_dir() {
            home.push(&path_str[2..]);
            return home;
        }
    }
    PathBuf::from(path_str)
}
