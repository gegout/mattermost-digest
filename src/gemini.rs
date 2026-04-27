// MIT License
// Copyright (c) 2026 Cedric Gegout

use reqwest::Client;
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::{get_config_dir, Config};
use crate::error::AppError;

/// Returns the expanded path for a file in the config directory.
fn get_config_file_path(filename: &str) -> PathBuf {
    let mut path = get_config_dir();
    path.push(filename);
    let expanded = crate::config::expand_tilde(&path.to_string_lossy());
    tracing::info!("Resolved config file path for '{}' to: {:?}", filename, expanded);
    expanded
}

/// Loads the requester context from `context.txt`.
/// Returns an empty string if the file doesn't exist or is empty.
fn load_context_text() -> String {
    tracing::info!("Attempting to load context from context.txt...");
    let path = get_config_file_path("context.txt");
    match fs::read_to_string(&path) {
        Ok(text) => {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                tracing::warn!("context.txt at {:?} is empty.", path);
            } else {
                tracing::info!("Successfully loaded context.txt ({} bytes)", trimmed.len());
            }
            trimmed
        }
        Err(e) => {
            tracing::warn!(
                "Could not read context.txt at {:?}, continuing with empty context: {}",
                path,
                e
            );
            String::new()
        }
    }
}

/// Loads the prior continuity history from `history.txt`.
/// Returns an empty string if the file doesn't exist or is empty.
fn load_history_text() -> String {
    tracing::info!("Attempting to load history from history.txt...");
    let path = get_config_file_path("history.txt");
    match fs::read_to_string(&path) {
        Ok(text) => {
            let trimmed = text.trim().to_string();
            if trimmed.is_empty() {
                tracing::warn!("history.txt at {:?} is empty.", path);
            } else {
                tracing::info!("Successfully loaded history.txt ({} bytes)", trimmed.len());
            }
            trimmed
        }
        Err(e) => {
            tracing::warn!(
                "Could not read history.txt at {:?}, treating as missing: {}",
                path,
                e
            );
            String::new()
        }
    }
}

/// Generates a compact continuity memory for the next digest pass
/// and saves it to `history.txt`.
async fn generate_history_from_digest(config: &Config, digest_markdown: &str) -> Result<(), AppError> {
    tracing::info!("Generating new history for the next cycle...");
    tracing::info!("Building history generation prompt...");
    let prompt = build_history_prompt(digest_markdown);
    
    tracing::info!("Calling Gemini to generate new history...");
    let history_content = call_gemini_text(config, &prompt).await?;
    
    tracing::info!("History generation completed, preparing to save to disk...");
    let path = get_config_file_path("history.txt");
    
    // Ensure parent directory exists, just in case
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            tracing::info!("Creating parent directory for history file at {:?}", parent);
            let _ = fs::create_dir_all(parent);
        }
    }

    fs::write(&path, history_content.trim()).map_err(|e| {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Failed to write history.txt: {}", e),
        ))
    })?;
    
    tracing::info!("Successfully saved new history to {:?}", path);
    Ok(())
}

/// Builds the main summary prompt, injecting context and history.
fn build_summary_prompt(
    config: &Config,
    digest_markdown: &str,
    context_text: &str,
    history_text: &str,
) -> String {
    tracing::info!("Constructing the main summary prompt...");
    let mut prompt = String::new();

    prompt.push_str("System intent:\n");
    prompt.push_str("You are preparing an executive digest from internal chat logs.\n");
    prompt.push_str("Your job is to identify what matters most for the requester, especially items relevant to their role, current priorities, and product leadership responsibilities.\n\n");

    prompt.push_str("Inputs:\n");

    prompt.push_str("- Requester context: ");
    if context_text.is_empty() {
        tracing::info!("Injecting empty context to summary prompt");
        prompt.push_str("None provided.\n");
    } else {
        tracing::info!("Injecting requester context to summary prompt");
        prompt.push_str("\n<context>\n");
        prompt.push_str(context_text);
        prompt.push_str("\n</context>\n");
    }

    prompt.push_str("- Prior continuity history: ");
    if history_text.is_empty() {
        tracing::info!("Injecting empty history to summary prompt");
        prompt.push_str("None available.\n");
    } else {
        tracing::info!("Injecting prior continuity history to summary prompt");
        prompt.push_str("\n<history>\n");
        prompt.push_str(history_text);
        prompt.push_str("\n</history>\n");
    }

    prompt.push_str("\nInstructions:\n");
    prompt.push_str(&format!(
        "- The requester identity and username is '{}'\n",
        config.mattermost.my_username
    ));
    prompt.push_str("- Use the requester context to understand who the requester is and what is likely relevant\n");
    prompt.push_str("- Use the prior history only as continuity context, not as a substitute for the current logs\n");
    prompt.push_str("- Focus first on items directly related to the requester, then product management, then broader relevance\n");
    prompt.push_str("- Prefer signal over exhaustiveness\n");
    prompt.push_str("- Be concrete\n");
    prompt.push_str("- Keep the output readable and concise\n");
    prompt.push_str("- Output valid Markdown\n");
    prompt.push_str("- Output exactly these 4 sections with these headings:\n\n");

    prompt.push_str("## What is important for my role, or related to me\n");
    prompt.push_str("## What are important items for Product Management\n");
    prompt.push_str("## What is important for the others\n");
    prompt.push_str("## What is just FYI\n\n");

    prompt.push_str("Current chat logs:\n");
    prompt.push_str(digest_markdown);

    tracing::info!("Main summary prompt construction complete ({} bytes)", prompt.len());
    prompt
}

/// Builds the dedicated prompt for generating continuity history.
fn build_history_prompt(digest_markdown: &str) -> String {
    tracing::info!("Constructing the dedicated history generation prompt...");
    let prompt = format!(
        "You are creating a compact continuity memory for the next digest pass.\n\
         Based on the current chat logs, produce a short, high-signal readout that will help a later summarization understand ongoing context.\n\
         Include:\n\
         - ongoing topics\n\
         - open decisions\n\
         - unresolved actions or questions\n\
         - people, teams, and projects that matter\n\
         - signals specifically relevant to the requester\n\
         - signals relevant to product management\n\
         Be concise.\n\
         Do not rewrite the full digest.\n\
         Produce compact Markdown that is useful as prior context for the next run.\n\n\
         Current chat logs:\n\
         {}",
        digest_markdown
    );
    tracing::info!("History generation prompt construction complete ({} bytes)", prompt.len());
    prompt
}

/// Makes a generic text-generation call to the Gemini API for a specific model.
async fn call_gemini_text_single_model(config: &Config, model: &str, prompt: &str) -> Result<String, AppError> {
    tracing::info!("Preparing to call Gemini API (model: {})...", model);
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, config.gemini.api_key
    );

    let payload = json!({
        "contents": [{
            "parts": [{"text": prompt}]
        }]
    });

    let client = Client::new();
    let max_retries = config.gemini.max_retries;
    let mut attempt = 0;

    loop {
        attempt += 1;
        tracing::info!("Sending HTTP POST request to Gemini API (model: {}, attempt: {}/{})...", model, attempt, max_retries);
        
        match client.post(&url).json(&payload).send().await {
            Ok(response) => {
                if response.status().is_success() {
                    tracing::info!("Received successful response from Gemini API, parsing JSON...");
                    let response_data: serde_json::Value = response.json().await.map_err(|e| {
                        AppError::Io(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to parse Gemini response: {}", e),
                        ))
                    })?;

                    tracing::info!("Extracting text content from Gemini response payload...");
                    if let Some(text) = response_data["candidates"][0]["content"]["parts"][0]["text"].as_str() {
                        tracing::info!("Successfully extracted {} characters from Gemini response", text.len());
                        return Ok(text.to_string());
                    } else {
                        tracing::error!("Gemini response payload did not contain expected text field");
                        return Err(AppError::Io(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            "Failed to extract text from Gemini response".to_string(),
                        )));
                    }
                } else {
                    let status = response.status();
                    let error_body = response.text().await.unwrap_or_default();
                    tracing::error!("Gemini API responded with HTTP {}: {}", status, error_body);
                    
                    if (status.as_u16() == 503 || status.as_u16() == 429) && attempt < max_retries {
                        let delay_secs = (config.gemini.retry_delay_base_seconds as u64) * 2_u64.pow((attempt - 1) as u32);
                        tracing::warn!("API is currently unavailable/rate-limited. Retrying in {} seconds...", delay_secs);
                        tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                        continue;
                    }
                    
                    return Err(AppError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        format!("Gemini API error ({}): {}", status, error_body),
                    )));
                }
            }
            Err(e) => {
                tracing::error!("HTTP request to Gemini API failed: {}", e);
                if attempt < max_retries {
                    let delay_secs = (config.gemini.retry_delay_base_seconds as u64) * 2_u64.pow((attempt - 1) as u32);
                    tracing::warn!("Network error. Retrying in {} seconds...", delay_secs);
                    tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                    continue;
                }
                return Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    format!("Failed to send to Gemini: {}", e),
                )));
            }
        }
    }
}

/// Makes a text-generation call with fallback logic to another model if the primary fails.
async fn call_gemini_text(config: &Config, prompt: &str) -> Result<String, AppError> {
    match call_gemini_text_single_model(config, &config.gemini.model, prompt).await {
        Ok(result) => Ok(result),
        Err(e) => {
            tracing::warn!("Primary model ({}) failed completely: {}. Switching to fallback model: {}", config.gemini.model, e, config.gemini.fallback_model);
            call_gemini_text_single_model(config, &config.gemini.fallback_model, prompt).await
        }
    }
}

/// Main entrypoint for summarization: generates a summary and advances the rolling history.
pub async fn summarize_digest(config: &Config, digest_markdown: &str) -> Result<String, AppError> {
    tracing::info!(
        "Initiating Gemini summarization pipeline using model {}...",
        config.gemini.model
    );

    // 1. Read existing files
    tracing::info!("Step 1: Reading existing context and history files...");
    let context_text = load_context_text();
    let history_text = load_history_text();

    // 2. Build the main summary prompt
    tracing::info!("Step 2: Building main summary prompt with loaded contexts...");
    let summary_prompt = build_summary_prompt(config, digest_markdown, &context_text, &history_text);

    // 3. Call Gemini for the main summary
    tracing::info!("Step 3: Executing API call for main summary...");
    let summary_result = call_gemini_text(config, &summary_prompt).await?;
    tracing::info!("Successfully received main summary from Gemini API.");

    // 4. Generate new history for the *next* run based on the *current* logs
    tracing::info!("Step 4: Executing background history generation for next run...");
    if let Err(e) = generate_history_from_digest(config, digest_markdown).await {
        tracing::warn!("Failed to generate new history for next run: {}. Continuing anyway.", e);
    } else {
        tracing::info!("Background history generation successfully completed.");
    }

    // 5. Return the summary
    tracing::info!("Step 5: Summarization pipeline finished returning final payload.");
    Ok(summary_result)
}

pub async fn test_connection(config: &Config) -> Result<(), AppError> {
    tracing::info!("Testing Gemini API connection...");
    tracing::info!("Dispatching generic OK test message...");
    let response = call_gemini_text(
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
