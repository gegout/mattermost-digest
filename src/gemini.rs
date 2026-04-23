// MIT License
// Copyright (c) 2026 Cedric Gegout

use reqwest::Client;
use serde_json::json;

use crate::config::Config;
use crate::error::AppError;

pub async fn summarize_digest(config: &Config, digest_markdown: &str) -> Result<String, AppError> {
    tracing::info!(
        "Sending digest to Gemini API for summarization using model {}...",
        config.gemini.model
    );

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        config.gemini.model, config.gemini.api_key
    );

    let prompt = format!(
        "Summarize the following chat logs. Provide the summary using Markdown formatting with 4 clear sections:\n\
         1. What is important for me or related to me (my username is '{}')\n\
         2. What are important items for the Product Management\n\
         3. What is important for the others\n\
         4. What is just FYI\n\n\
         Chat Logs:\n{}",
        config.mattermost.my_username, digest_markdown
    );

    let payload = json!({
        "contents": [{
            "parts": [{"text": prompt}]
        }]
    });

    let client = Client::new();
    let response = client.post(&url).json(&payload).send().await.map_err(|e| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to send to Gemini: {}", e),
        ))
    })?;

    if !response.status().is_success() {
        let status = response.status();
        let error_body = response.text().await.unwrap_or_default();
        return Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Gemini API error ({}): {}", status, error_body),
        )));
    }

    let response_data: serde_json::Value = response.json().await.map_err(|e| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to parse Gemini response: {}", e),
        ))
    })?;

    if let Some(text) = response_data["candidates"][0]["content"]["parts"][0]["text"].as_str() {
        tracing::info!("Successfully received summary from Gemini API.");
        Ok(text.to_string())
    } else {
        Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            "Failed to extract text from Gemini response".to_string(),
        )))
    }
}

pub async fn test_connection(config: &Config) -> Result<(), AppError> {
    tracing::info!("Testing Gemini API connection...");
    let response = summarize_digest(
        config,
        "This is a test message. Please respond with exactly 'OK'.",
    )
    .await?;
    tracing::info!(
        "Gemini test successful! Response length: {} chars",
        response.len()
    );
    Ok(())
}
