// MIT License
// Copyright (c) 2026 Cedric Gegout

use thiserror::Error;

/// Enumeration of possible errors across the Mattermost Digest application.
#[derive(Error, Debug)]
pub enum AppError {
    /// Errors related to parsing or loading the configuration file.
    #[error("Configuration error: {0}")]
    Config(String),

    /// Errors returned by the Mattermost API client.
    #[error("Mattermost API error: {0}")]
    Mattermost(String),

    /// Errors encountered during Gmail API authentication or email sending.
    #[error("Gmail API error: {0}")]
    Gmail(String),

    /// Standard I/O errors (e.g., reading/writing files).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Errors encountered during JSON serialization or deserialization.
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    /// Network and HTTP-level errors via reqwest.
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    /// Errors in generating or formatting the resulting markdown string.
    #[error("Markdown formatting error: {0}")]
    Markdown(String),
}
