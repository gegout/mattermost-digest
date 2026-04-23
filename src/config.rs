// MIT License
// Copyright (c) 2026 Cedric Gegout

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::fs;
use crate::error::AppError;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    pub mattermost: MattermostConfig,
    pub gmail: GmailConfig,
    pub gemini: GeminiConfig,
    pub output: OutputConfig,
    pub logging: LoggingConfig,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct MattermostConfig {
    #[serde(default = "default_base_url")]
    pub base_url: String,
    pub personal_token: String,
    #[serde(default = "default_lookback_hours")]
    pub lookback_hours: u32,
    #[serde(default = "default_request_timeout_seconds")]
    pub request_timeout_seconds: u64,
    #[serde(default = "default_per_page")]
    pub per_page: u32,
    #[serde(default = "default_my_username")]
    pub my_username: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GmailConfig {
    pub client_secret_path: String,
    pub token_cache_path: String,
    pub from_email: String,
    pub to_email: String,
    #[serde(default = "default_email_subject")]
    pub email_subject: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct GeminiConfig {
    pub api_key: String,
    #[serde(default = "default_gemini_model")]
    pub model: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct OutputConfig {
    pub markdown_path: String,
    #[serde(default = "default_false")]
    pub include_empty_channels: bool,
    #[serde(default = "default_max_posts")]
    pub max_posts_per_channel: u32,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
}

fn default_base_url() -> String { "https://chat.canonical.com".to_string() }
fn default_my_username() -> String { "cgegout".to_string() }
fn default_lookback_hours() -> u32 { 24 }
fn default_request_timeout_seconds() -> u64 { 30 }
fn default_per_page() -> u32 { 200 }
fn default_gemini_model() -> String { "gemini-2.5-flash".to_string() }
fn default_email_subject() -> String { "Mattermost Digest".to_string() }
fn default_false() -> bool { false }
fn default_max_posts() -> u32 { 500 }
fn default_log_level() -> String { "info".to_string() }

impl Config {
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

pub fn get_config_dir() -> PathBuf {
    let mut path = dirs::config_dir().unwrap_or_else(|| PathBuf::from("~/.config"));
    path.push("mattermost-digest");
    path
}

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

pub fn get_config_path() -> PathBuf {
    get_config_dir().join("config.toml")
}

pub fn expand_tilde(path_str: &str) -> PathBuf {
    if path_str.starts_with("~/") {
        if let Some(mut home) = dirs::home_dir() {
            home.push(&path_str[2..]);
            return home;
        }
    }
    PathBuf::from(path_str)
}
