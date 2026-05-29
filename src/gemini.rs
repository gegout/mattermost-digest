// MIT License
// Copyright (c) 2026 Cedric Gegout

//! # Gemini API Caller
//!
//! This module handles all HTTP communication with the Gemini `generateContent` endpoint.
//! It does NOT make model selection decisions — that responsibility belongs entirely to
//! `crate::gemini_model_ranking`. All model choices flow through that module so that
//! `run` mode and `bot` mode always use the same ranking strategy.

use reqwest::{Client, StatusCode};
use serde_json::json;
use std::fmt;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::config::{get_config_dir, Config};
use crate::error::AppError;
// Re-export GeminiModelInfo so callers that previously imported it from gemini still compile.
pub use crate::gemini_model_ranking::GeminiModelInfo;
use crate::gemini_model_ranking::select_model_for_request;

/// Categories of errors returned by the Gemini API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeminiErrorCategory {
    /// 503 / high-demand / temporary server issue — retry same model.
    TransientUnavailable,
    /// 429 rate limited / throttled — retry same model with backoff.
    Throttled,
    /// 404 model not found or deprecated — switch model immediately.
    ModelNotFound,
    /// Model exists but doesn't support generateContent — switch immediately.
    UnsupportedMethod,
    /// 401/403 bad API key or permission — fail fast, no model fallback.
    AuthError,
    /// Quota or credits exhausted — may allow fallback to another model.
    QuotaExhausted,
    /// Other 4xx client error.
    ClientError,
    /// Other 5xx server error — retry same model.
    ServerError,
}

impl fmt::Display for GeminiErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TransientUnavailable => write!(f, "Transiently unavailable (503)"),
            Self::Throttled => write!(f, "Rate limited / Throttled (429)"),
            Self::ModelNotFound => write!(f, "Model not found (404)"),
            Self::UnsupportedMethod => write!(f, "Method not supported by model"),
            Self::AuthError => write!(f, "Authentication or permission error"),
            Self::QuotaExhausted => write!(f, "API Quota or credits exhausted"),
            Self::ClientError => write!(f, "Other client error"),
            Self::ServerError => write!(f, "Other server error"),
        }
    }
}

/// A record of a single attempt on a specific model (used to build the final failure report).
#[derive(Debug, Clone)]
pub struct GeminiAttemptReport {
    pub model_name: String,
    pub attempt_number: u32,
    pub result: Result<(), GeminiErrorCategory>,
    pub details: String,
}

/// An mpsc sender used to push human-readable status strings to a Telegram progress message
/// while a long Gemini operation is running.
pub type GeminiProgressSender = mpsc::Sender<String>;

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
    use_html: bool,
) -> String {
    tracing::info!("Constructing the main summary prompt (HTML={})...", use_html);
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

    if use_html {
        prompt.push_str("- Output valid Telegram-compatible HTML (using <b>, <i>, <code> tags).\n");
        prompt.push_str("- DO NOT use markdown symbols like **, *, or ###.\n");
        prompt.push_str("- Output exactly these 4 sections with these bold headings:\n\n");
        prompt.push_str("<b>What is important for my role, or related to me</b>\n");
        prompt.push_str("<b>What are important items for Product Management</b>\n");
        prompt.push_str("<b>What is important for the others</b>\n");
        prompt.push_str("<b>What is just FYI</b>\n\n");
    } else {
        prompt.push_str("- Output valid Markdown\n");
        prompt.push_str("- Output exactly these 4 sections with these markdown headings:\n\n");
        prompt.push_str("## What is important for my role, or related to me\n");
        prompt.push_str("## What are important items for Product Management\n");
        prompt.push_str("## What is important for the others\n");
        prompt.push_str("## What is just FYI\n\n");
    }

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

/// Classifies a Gemini API response into an error category.
fn extract_gemini_error_category(status: StatusCode, body: &str) -> GeminiErrorCategory {
    let lower_body = body.to_lowercase();
    let is_quota = lower_body.contains("quota") || lower_body.contains("exhausted") || lower_body.contains("limit");

    match status {
        StatusCode::SERVICE_UNAVAILABLE => GeminiErrorCategory::TransientUnavailable,
        StatusCode::TOO_MANY_REQUESTS => {
            if is_quota {
                GeminiErrorCategory::QuotaExhausted
            } else {
                GeminiErrorCategory::Throttled
            }
        }
        StatusCode::NOT_FOUND => GeminiErrorCategory::ModelNotFound,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => GeminiErrorCategory::AuthError,
        s if s.is_server_error() => GeminiErrorCategory::ServerError,
        _ => {
            // Check for quota messages in body even if status is not 429
            if is_quota {
                GeminiErrorCategory::QuotaExhausted
            } else if lower_body.contains("not found") || lower_body.contains("unsupported") {
                GeminiErrorCategory::ModelNotFound
            } else {
                GeminiErrorCategory::ClientError
            }
        }
    }
}

/// Returns true if the error category justifies retrying the SAME model.
fn is_retryable_gemini_error(cat: GeminiErrorCategory) -> bool {
    matches!(
        cat,
        GeminiErrorCategory::TransientUnavailable
            | GeminiErrorCategory::Throttled
            | GeminiErrorCategory::ServerError
    )
}

/// Returns true if the error category means the model itself is definitively invalid —
/// i.e. we should switch to a different model immediately without wasting retry attempts.
fn is_model_invalid_error(cat: GeminiErrorCategory) -> bool {
    matches!(
        cat,
        GeminiErrorCategory::ModelNotFound | GeminiErrorCategory::UnsupportedMethod
    )
}

/// Makes a single `generateContent` HTTP call for the given model and prompt.
///
/// Returns `Ok(text)` on success, or `Err((category, details))` on failure where
/// `category` tells the caller how to react (retry / switch model / fail fast).
///
/// The model name is normalised to always include the `models/` prefix expected by
/// the v1beta API path.
async fn call_gemini_single_model(
    config: &Config,
    model: &str,
    prompt: &str,
    attempt: u32,
) -> Result<String, (GeminiErrorCategory, String)> {
    // Normalise model name: the API URL requires the full "models/<name>" path.
    let full_model_path = if model.starts_with("models/") {
        model.to_string()
    } else {
        format!("models/{}", model)
    };

    tracing::info!(
        "Calling Gemini API (model: {}, attempt: {}/{})...",
        full_model_path,
        attempt,
        config.gemini.max_attempts_per_model
    );

    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/{}:generateContent?key={}",
        full_model_path, config.gemini.api_key
    );

    let payload = json!({
        "contents": [{
            "parts": [{"text": prompt}]
        }]
    });

    let client = Client::new();
    match client.post(&url).json(&payload).send().await {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                match response.json::<serde_json::Value>().await {
                    Ok(v) => {
                        if let Some(text) =
                            v["candidates"][0]["content"]["parts"][0]["text"].as_str()
                        {
                            Ok(text.to_string())
                        } else {
                            Err((
                                GeminiErrorCategory::ServerError,
                                "Response JSON missing text content".to_string(),
                            ))
                        }
                    }
                    Err(e) => Err((
                        GeminiErrorCategory::ServerError,
                        format!("Failed to parse JSON: {}", e),
                    )),
                }
            } else {
                let body = response.text().await.unwrap_or_default();
                let cat = extract_gemini_error_category(status, &body);
                Err((cat, body))
            }
        }
        Err(e) => Err((
            GeminiErrorCategory::TransientUnavailable,
            format!("Network error: {}", e),
        )),
    }
}

/// Core Gemini call orchestrator.
///
/// This function is the **single entry point** for all Gemini text generation requests,
/// used by both `run` mode and `bot` mode. It implements the full retry + fallback loop:
///
/// 1. Ask the ranking module for the best available model (initial selection).
/// 2. Try that model up to `max_attempts_per_model` times.
///    - On transient errors (503, 429): backoff and retry the same model.
///    - On definitive invalid-model errors (404, unsupported): switch immediately.
///    - On auth errors: fail fast without trying any fallback.
/// 3. If the model is exhausted, ask the ranking module for the next best model
///    (passing the failed model and all already-tried models as context).
/// 4. Repeat until success or `max_models_to_try` models have been tried.
/// 5. If all models fail, return a detailed failure report.
///
/// The `progress_tx` channel allows the Telegram bot to update its progress message
/// with user-friendly status while the operation runs in the background.
pub async fn call_gemini_with_fallbacks(
    config: &Config,
    prompt: &str,
    progress_tx: Option<GeminiProgressSender>,
) -> Result<String, AppError> {
    let max_models = config.gemini.max_models_to_try as usize;
    let max_attempts = config.gemini.max_attempts_per_model;

    let mut tried_models: Vec<String> = Vec::new();
    let mut attempt_reports: Vec<GeminiAttemptReport> = Vec::new();
    let mut last_failed_model: Option<String> = None;

    tracing::info!(
        "Starting Gemini request pipeline (max_models={}, max_attempts_per_model={})",
        max_models,
        max_attempts
    );

    while tried_models.len() < max_models {
        // --- Model selection via the centralized ranking strategy ---
        // We pass the last failed model and all already-tried models so the ranking
        // module can exclude them and return the next best available option.
        let failed_ref = last_failed_model.as_deref();
        let model_info = match select_model_for_request(config, failed_ref, &tried_models).await {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!("Model selection returned no candidate: {}", e);
                break; // No more models available — exit the outer loop
            }
        };

        let current_model = model_info.name.clone();
        tried_models.push(current_model.clone());
        tracing::info!(
            "Using model: '{}' (attempt {}/{})",
            current_model,
            tried_models.len(),
            max_models
        );

        // Notify Telegram bot of model switch (only on fallback, not first selection)
        if tried_models.len() > 1 {
            if let Some(ref tx) = progress_tx {
                let short = model_info.short_name().to_string();
                let _ = tx
                    .send(format!(
                        "⚠️ Switching to fallback model <code>{}</code>...",
                        short
                    ))
                    .await;
            }
        }

        // --- Per-model retry loop ---
        let mut model_attempts: u32 = 0;
        let model_succeeded = false;

        loop {
            model_attempts += 1;

            match call_gemini_single_model(config, &current_model, prompt, model_attempts).await {
                Ok(text) => {
                    tracing::info!(
                        "✅ Gemini success with model '{}' on attempt {}",
                        current_model,
                        model_attempts
                    );
                    return Ok(text);
                }
                Err((cat, details)) => {
                    attempt_reports.push(GeminiAttemptReport {
                        model_name: current_model.clone(),
                        attempt_number: model_attempts,
                        result: Err(cat),
                        details: details.clone(),
                    });

                    tracing::warn!(
                        "Model '{}' failed attempt {}/{} — category: {}, details: {}",
                        current_model,
                        model_attempts,
                        max_attempts,
                        cat,
                        &details[..details.len().min(200)]
                    );

                    // Auth errors are account-wide — no model fallback will help.
                    if cat == GeminiErrorCategory::AuthError {
                        return Err(AppError::Io(std::io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Gemini authentication error (fail fast): {}", details),
                        )));
                    }

                    // For definitive model errors (404, unsupported), skip remaining retries.
                    // Wasting retry quota on a model that is definitively broken is pointless.
                    if is_model_invalid_error(cat) {
                        tracing::warn!(
                            "Model '{}' is definitively invalid ({}). Switching immediately.",
                            current_model,
                            cat
                        );
                        break;
                    }

                    // For transient errors, retry the same model with backoff.
                    if model_attempts < max_attempts && is_retryable_gemini_error(cat) {
                        let delay = (config.gemini.retry_delay_base_seconds as u64)
                            * 2_u64.pow(model_attempts - 1);
                        tracing::info!(
                            "Retrying model '{}' in {} seconds (attempt {}/{})...",
                            current_model,
                            delay,
                            model_attempts + 1,
                            max_attempts
                        );
                        tokio::time::sleep(Duration::from_secs(delay)).await;
                        continue;
                    }

                    // Exhausted retries for this model without success.
                    tracing::warn!(
                        "Model '{}' exhausted all {} attempts. Moving to next model.",
                        current_model,
                        max_attempts
                    );
                    break;
                }
            }
        }

        if model_succeeded {
            break;
        }

        // Record which model failed so the ranking module can exclude it next iteration.
        last_failed_model = Some(current_model.clone());
    }

    // --- Build final error report ---
    let mut error_msg = format!(
        "All Gemini models failed. Tried {} model(s) total.\n\n",
        tried_models.len()
    );
    for (i, report) in attempt_reports.iter().enumerate() {
        error_msg.push_str(&format!(
            "{}. Model: {}, Attempt: {}, Error: {}\n",
            i + 1,
            report.model_name,
            report.attempt_number,
            report.result.err().unwrap_or(GeminiErrorCategory::ServerError)
        ));
    }

    tracing::error!("{}", error_msg);
    Err(AppError::Io(std::io::Error::new(
        std::io::ErrorKind::Other,
        error_msg,
    )))
}

/// Makes a text-generation call with the full ranking strategy.
/// All internal callers within this module use this function.
async fn call_gemini_text(config: &Config, prompt: &str) -> Result<String, AppError> {
    call_gemini_with_fallbacks(config, prompt, None).await
}

/// Public entry-point for ad-hoc Gemini text generation from modules such as the Telegram bot.
/// Applies the same primary → fallback model strategy as all other calls.
pub async fn call_gemini_text_for_bot(config: &Config, prompt: &str) -> Result<String, AppError> {
    call_gemini_text(config, prompt).await
}

/// Main entrypoint for custom summarization: generates a summary without modifying history.
pub async fn summarize_custom_digest(
    config: &Config, 
    digest_markdown: &str,
    custom_context: Option<String>,
    custom_history: Option<String>,
    use_html: bool,
    progress_tx: Option<GeminiProgressSender>,
) -> Result<String, AppError> {
    tracing::info!("Initiating custom Gemini summarization pipeline (HTML={}, Callback={})...", use_html, progress_tx.is_some());

    let context_text = if let Some(ctx) = custom_context {
        tracing::info!("Using OVERRIDDEN context:\n---\n{}\n---", ctx);
        ctx
    } else {
        let ctx = load_context_text();
        tracing::info!("Using context from disk:\n---\n{}\n---", ctx);
        ctx
    };

    let history_text = if let Some(hist) = custom_history {
        tracing::info!("Using OVERRIDDEN history:\n---\n{}\n---", hist);
        hist
    } else {
        let hist = load_history_text();
        tracing::info!("Using history from disk:\n---\n{}\n---", hist);
        hist
    };

    let summary_prompt = build_summary_prompt(config, digest_markdown, &context_text, &history_text, use_html);

    let summary_result = call_gemini_with_fallbacks(config, &summary_prompt, progress_tx).await?;
    tracing::info!("Successfully received custom summary from Gemini API.");

    // Note: intentionally skipping history generation to avoid contaminating the main history flow.

    Ok(summary_result)
}

/// Main entrypoint for summarization: generates a summary and advances the rolling history.
pub async fn summarize_digest(config: &Config, digest_markdown: &str) -> Result<String, AppError> {
    tracing::info!("Initiating Gemini summarization pipeline...");

    // 1. Read existing files
    tracing::info!("Step 1: Reading existing context and history files from disk...");
    let context_text = load_context_text();
    let history_text = load_history_text();
    tracing::info!("Context loaded:\n---\n{}\n---", context_text);
    tracing::info!("History loaded:\n---\n{}\n---", history_text);

    // 2. Build the main summary prompt
    tracing::info!("Step 2: Building main summary prompt with loaded contexts...");
    let summary_prompt = build_summary_prompt(config, digest_markdown, &context_text, &history_text, false);

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Tests for `extract_gemini_error_category` — the only branching logic that still
    /// lives in this module. Model ranking and selection tests live in `gemini_model_ranking`.
    #[test]
    fn test_error_categorization() {
        assert_eq!(
            extract_gemini_error_category(StatusCode::SERVICE_UNAVAILABLE, ""),
            GeminiErrorCategory::TransientUnavailable
        );
        assert_eq!(
            extract_gemini_error_category(StatusCode::TOO_MANY_REQUESTS, ""),
            GeminiErrorCategory::Throttled
        );
        assert_eq!(
            extract_gemini_error_category(StatusCode::NOT_FOUND, ""),
            GeminiErrorCategory::ModelNotFound
        );
        assert_eq!(
            extract_gemini_error_category(StatusCode::UNAUTHORIZED, ""),
            GeminiErrorCategory::AuthError
        );
        assert_eq!(
            extract_gemini_error_category(StatusCode::OK, "quota exceeded"),
            GeminiErrorCategory::QuotaExhausted
        );
        assert_eq!(
            extract_gemini_error_category(StatusCode::TOO_MANY_REQUESTS, "quota exceeded"),
            GeminiErrorCategory::QuotaExhausted
        );
        assert_eq!(
            extract_gemini_error_category(StatusCode::TOO_MANY_REQUESTS, "rate limit exceeded"),
            GeminiErrorCategory::QuotaExhausted
        );
        assert_eq!(
            extract_gemini_error_category(StatusCode::TOO_MANY_REQUESTS, "too many requests"),
            GeminiErrorCategory::Throttled
        );
        assert_eq!(
            extract_gemini_error_category(StatusCode::OK, "Model models/foo not found"),
            GeminiErrorCategory::ModelNotFound
        );
    }

    #[test]
    fn test_retryable_logic() {
        assert!(is_retryable_gemini_error(GeminiErrorCategory::TransientUnavailable));
        assert!(is_retryable_gemini_error(GeminiErrorCategory::Throttled));
        assert!(is_retryable_gemini_error(GeminiErrorCategory::ServerError));
        assert!(!is_retryable_gemini_error(GeminiErrorCategory::ModelNotFound));
        assert!(!is_retryable_gemini_error(GeminiErrorCategory::AuthError));
    }

    #[test]
    fn test_model_invalid_logic() {
        assert!(is_model_invalid_error(GeminiErrorCategory::ModelNotFound));
        assert!(is_model_invalid_error(GeminiErrorCategory::UnsupportedMethod));
        assert!(!is_model_invalid_error(GeminiErrorCategory::TransientUnavailable));
        assert!(!is_model_invalid_error(GeminiErrorCategory::AuthError));
    }
}
