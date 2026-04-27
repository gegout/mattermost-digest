// MIT License
// Copyright (c) 2026 Cedric Gegout

use chrono::Utc;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};

use crate::config::Config;
use crate::digest;
use crate::error::AppError;
use crate::gemini;
use crate::gmail;
use crate::mattermost::MattermostClient;
use crate::system_status::get_system_status;
use crate::telegram_commands::{parse_command, Command, ConversationState, CustomDigestStep, DigestOverrides, StateManager};
use crate::telegram_format::{escape_html, format_error, format_success, format_system_status};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sends a Telegram message to a specific chat, formatting it with the
/// parse mode configured for the bot (typically HTML).
async fn send_message(client: &Client, token: &str, chat_id: i64, text: &str, parse_mode: &str) {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
    let payload = json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": parse_mode,
    });
    if let Err(e) = client.post(&url).json(&payload).send().await {
        tracing::error!("Failed to send Telegram message to {}: {}", chat_id, e);
    }
}

/// Converts Markdown text to plain HTML using pulldown-cmark (same as the email pipeline).
fn markdown_to_html(markdown: &str) -> String {
    let parser = pulldown_cmark::Parser::new(markdown);
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    html
}

// ---------------------------------------------------------------------------
// Main bot loop
// ---------------------------------------------------------------------------

/// Starts the long-polling Telegram bot loop. This runs indefinitely and handles
/// incoming messages, dispatching them to the appropriate command handlers.
pub async fn run_bot(config: Config) {
    let tconfig = match config.telegram.as_ref() {
        Some(t) => t.clone(),
        None => {
            tracing::error!("No [telegram] section in config.toml – cannot start bot mode.");
            return;
        }
    };

    let client = Client::builder()
        .timeout(Duration::from_secs(tconfig.request_timeout_seconds))
        .build()
        .expect("Failed to build HTTP client for Telegram");

    let mut offset = 0i64;
    let mut state_manager = StateManager::new();

    tracing::info!(
        "Telegram bot started. Allowed user IDs: {:?}",
        tconfig.allowed_user_ids
    );

    loop {
        // getUpdates with long-poll timeout
        let url = format!(
            "https://api.telegram.org/bot{}/getUpdates?offset={}&timeout={}",
            tconfig.bot_token, offset, tconfig.poll_interval_seconds
        );

        match client.get(&url).send().await {
            Ok(resp) => {
                if let Ok(data) = resp.json::<Value>().await {
                    if let Some(updates) = data.get("result").and_then(|r| r.as_array()) {
                        for update in updates {
                            // Advance the offset so we don't re-process this update.
                            if let Some(id) = update.get("update_id").and_then(|i| i.as_i64()) {
                                offset = id + 1;
                            }
                            if let Some(msg) = update.get("message") {
                                handle_message(&client, &config, msg, &mut state_manager).await;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Telegram poll error: {}. Retrying in {}s…", e, tconfig.poll_interval_seconds);
            }
        }

        sleep(Duration::from_secs(tconfig.poll_interval_seconds)).await;
    }
}

// ---------------------------------------------------------------------------
// Message handler
// ---------------------------------------------------------------------------

/// Dispatches a single incoming Telegram message to the correct handler.
async fn handle_message(
    client: &Client,
    config: &Config,
    message: &Value,
    state_manager: &mut StateManager,
) {
    let tconfig = config.telegram.as_ref().unwrap();

    let chat_id = message
        .get("chat")
        .and_then(|c| c.get("id"))
        .and_then(|i| i.as_i64())
        .unwrap_or(0);

    let user_id = message
        .get("from")
        .and_then(|f| f.get("id"))
        .and_then(|i| i.as_u64())
        .unwrap_or(0);

    // Reject unauthorised users silently.
    if !tconfig.allowed_user_ids.contains(&user_id) {
        tracing::warn!("Rejected message from unauthorized user_id={}", user_id);
        return;
    }

    let text = match message.get("text").and_then(|t| t.as_str()) {
        Some(t) => t,
        None => return,
    };

    tracing::info!("Received message from user {}: {:?}", user_id, text);

    // -----------------------------------------------------------------------
    // Multi-step custom digest conversation
    // -----------------------------------------------------------------------
    if let Some(state) = state_manager.sessions.remove(&user_id) {
        // Allow cancellation at any step.
        if matches!(parse_command(text), Some(Command::Cancel)) {
            send_message(client, &tconfig.bot_token, chat_id, "🚫 <b>Custom digest cancelled.</b>", &tconfig.parse_mode).await;
            return;
        }
        handle_custom_digest_step(client, config, chat_id, user_id, text, state, state_manager).await;
        return;
    }

    // -----------------------------------------------------------------------
    // Top-level command dispatch
    // -----------------------------------------------------------------------
    match parse_command(text) {
        Some(Command::Status) => {
            tracing::info!("Handling /status command for user {}", user_id);
            let status = get_system_status();
            send_message(client, &tconfig.bot_token, chat_id, &format_system_status(&status), &tconfig.parse_mode).await;
        }

        Some(Command::Digest) => {
            tracing::info!("Handling /digest command for user {}", user_id);
            send_message(client, &tconfig.bot_token, chat_id, "⚙️ <b>Generating standard digest…</b> This may take a moment.", &tconfig.parse_mode).await;
            match run_standard_digest(config).await {
                Ok(_) => {
                    send_message(client, &tconfig.bot_token, chat_id, &format_success("Standard digest generated and emailed successfully!"), &tconfig.parse_mode).await;
                }
                Err(e) => {
                    tracing::error!("/digest failed: {}", e);
                    send_message(client, &tconfig.bot_token, chat_id, &format_error(&e.to_string()), &tconfig.parse_mode).await;
                }
            }
        }

        Some(Command::CustomDigest) => {
            tracing::info!("Handling /custom command for user {}", user_id);
            state_manager.sessions.insert(user_id, ConversationState::new());
            let prompt = "🛠 <b>Custom Digest Mode</b>\n\
                You can override individual inputs. Type <code>skip</code> to keep the default for any step.\n\n\
                ✍️ <b>Step 1/3 – Context override</b>\n\
                Provide custom context text (or <code>skip</code>):";
            send_message(client, &tconfig.bot_token, chat_id, prompt, &tconfig.parse_mode).await;
        }

        Some(Command::Cancel) => {
            send_message(client, &tconfig.bot_token, chat_id, "ℹ️ No active session to cancel.", &tconfig.parse_mode).await;
        }

        Some(Command::Unknown(_)) | None => {
            let help = "❓ <b>Unknown command.</b>\n\n\
                Available commands:\n\
                /status — Machine resource status\n\
                /digest — Generate and email standard digest\n\
                /custom — Generate custom digest (with overrides)\n\
                /cancel — Cancel an active custom digest session";
            send_message(client, &tconfig.bot_token, chat_id, help, &tconfig.parse_mode).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Custom digest – multi-step state machine
// ---------------------------------------------------------------------------

/// Advances a custom digest conversation by one step.
async fn handle_custom_digest_step(
    client: &Client,
    config: &Config,
    chat_id: i64,
    user_id: u64,
    text: &str,
    mut state: ConversationState,
    state_manager: &mut StateManager,
) {
    let tconfig = config.telegram.as_ref().unwrap();
    // "skip" means keep the default for this field.
    let input: Option<String> = if text.trim().to_lowercase() == "skip" {
        None
    } else {
        Some(text.trim().to_string())
    };

    match state.step {
        CustomDigestStep::AskContext => {
            state.overrides.context = input;
            state.step = CustomDigestStep::AskHistory;
            state_manager.sessions.insert(user_id, state);
            let prompt = "✍️ <b>Step 2/3 – History override</b>\n\
                Provide custom history text (or <code>skip</code>):";
            send_message(client, &tconfig.bot_token, chat_id, prompt, &tconfig.parse_mode).await;
        }

        CustomDigestStep::AskHistory => {
            state.overrides.history = input;
            state.step = CustomDigestStep::AskLookback;
            state_manager.sessions.insert(user_id, state);
            let prompt = "⏳ <b>Step 3/3 – Lookback hours override</b>\n\
                Provide a number of hours to look back (or <code>skip</code>):";
            send_message(client, &tconfig.bot_token, chat_id, prompt, &tconfig.parse_mode).await;
        }

        CustomDigestStep::AskLookback => {
            // Parse the numeric override if provided; reject non-numeric input.
            if let Some(ref val) = input {
                match val.parse::<u32>() {
                    Ok(hours) => state.overrides.lookback_hours = Some(hours),
                    Err(_) => {
                        let msg = "❌ Expected an integer for lookback hours. Please try again (or type <code>skip</code>):";
                        send_message(client, &tconfig.bot_token, chat_id, msg, &tconfig.parse_mode).await;
                        state.step = CustomDigestStep::AskLookback;
                        state_manager.sessions.insert(user_id, state);
                        return;
                    }
                }
            }

            // Summarise what will be overridden before running.
            let mut overrides_description = String::from("📋 <b>Overrides applied:</b>\n");
            overrides_description.push_str(&format!("• Context: {}\n", state.overrides.context.as_deref().map(|_| "✅ custom").unwrap_or("default")));
            overrides_description.push_str(&format!("• History: {}\n", state.overrides.history.as_deref().map(|_| "✅ custom").unwrap_or("default")));
            overrides_description.push_str(&format!("• Lookback hours: {}\n", state.overrides.lookback_hours.map_or("default".to_string(), |h| format!("✅ {}h", h))));
            overrides_description.push_str("\n⚙️ <b>Generating custom digest…</b>");
            send_message(client, &tconfig.bot_token, chat_id, &overrides_description, &tconfig.parse_mode).await;

            // Clone config and apply lookback override.
            let mut custom_config = config.clone();
            if let Some(h) = state.overrides.lookback_hours {
                custom_config.mattermost.lookback_hours = h;
            }

            match run_custom_digest(&custom_config, state.overrides).await {
                Ok(summary) => {
                    // Trim the summary to Telegram's 4096-char limit.
                    let truncated: String = summary.chars().take(3900).collect();
                    let msg = format!(
                        "✅ <b>Custom Digest Summary</b>\n\n{}{}",
                        escape_html(&truncated),
                        if summary.len() > 3900 { "\n\n<i>… (truncated)</i>" } else { "" }
                    );
                    send_message(client, &tconfig.bot_token, chat_id, &msg, &tconfig.parse_mode).await;
                }
                Err(e) => {
                    tracing::error!("Custom digest failed: {}", e);
                    send_message(client, &tconfig.bot_token, chat_id, &format_error(&e.to_string()), &tconfig.parse_mode).await;
                }
            }
        }

        CustomDigestStep::ReadyToRun => {}
    }
}

// ---------------------------------------------------------------------------
// Digest runners
// ---------------------------------------------------------------------------

/// Runs the standard Mattermost digest pipeline and sends the result by email.
/// Mirrors the `Commands::Run` logic in `main.rs` without any overrides.
async fn run_standard_digest(config: &Config) -> Result<(), AppError> {
    tracing::info!("Standard digest triggered from Telegram bot.");
    let mm_client = MattermostClient::new(&config.mattermost)?;
    let now = Utc::now();
    let result = digest::generate_digest(&mm_client, config, now).await?;

    let mut final_markdown = result.markdown.clone();
    if result.has_messages {
        match gemini::summarize_digest(config, &result.markdown).await {
            Ok(summary) => {
                final_markdown = format!("# Executive Summary\n\n{}\n\n---\n\n{}", summary, result.markdown);
            }
            Err(e) => {
                tracing::warn!("Gemini summarization failed during bot /digest: {}", e);
            }
        }
    }

    let html = markdown_to_html(&final_markdown);
    gmail::send_digest_email(&config.gmail, &html).await?;
    Ok(())
}

/// Runs the customised Mattermost digest pipeline and returns the raw AI summary text.
/// Intentionally does NOT send an email and does NOT overwrite `history.txt`.
async fn run_custom_digest(config: &Config, overrides: DigestOverrides) -> Result<String, AppError> {
    tracing::info!("Custom digest triggered from Telegram bot.");
    let mm_client = MattermostClient::new(&config.mattermost)?;
    let now = Utc::now();
    let result = digest::generate_digest(&mm_client, config, now).await?;

    let summary = gemini::summarize_custom_digest(
        config,
        &result.markdown,
        overrides.context,
        overrides.history,
    ).await?;

    Ok(summary)
}
