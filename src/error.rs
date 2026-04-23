// MIT License
// Copyright (c) 2026 Cedric Gegout

use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("Configuration error: {0}")]
    Config(String),

    #[error("Mattermost API error: {0}")]
    Mattermost(String),

    #[error("Gmail API error: {0}")]
    Gmail(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Markdown formatting error: {0}")]
    Markdown(String),
}
