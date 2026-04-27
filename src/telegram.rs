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
use crate::mattermost::MattermostClient;
use crate::system_status::get_system_status;
use crate::telegram_commands::{
    parse_command, Command, ConversationState, CustomDigestStep, DigestOverrides, StateManager,
};
use crate::telegram_format::{escape_html, format_error, format_system_status};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Sends a Telegram HTML message to a specific chat.
/// Long messages are automatically truncated to Telegram's 4 096-character limit.
async fn send_message(client: &Client, token: &str, chat_id: i64, text: &str, parse_mode: &str) {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
    let truncated: String = text.chars().take(4090).collect();
    let suffix = if text.len() > 4090 { "\n<i>…(truncated)</i>" } else { "" };
    let body = format!("{}{}", truncated, suffix);
    let payload = json!({
        "chat_id": chat_id,
        "text": body,
        "parse_mode": parse_mode,
    });
    if let Err(e) = client.post(&url).json(&payload).send().await {
        tracing::error!("Failed to send Telegram message to {}: {}", chat_id, e);
    }
}


// ---------------------------------------------------------------------------
// Main bot loop
// ---------------------------------------------------------------------------

/// Starts the long-polling Telegram bot loop. Runs indefinitely.
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
        let url = format!(
            "https://api.telegram.org/bot{}/getUpdates?offset={}&timeout={}",
            tconfig.bot_token, offset, tconfig.poll_interval_seconds
        );

        match client.get(&url).send().await {
            Ok(resp) => {
                if let Ok(data) = resp.json::<Value>().await {
                    if let Some(updates) = data.get("result").and_then(|r| r.as_array()) {
                        for update in updates {
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

    // Silently reject unauthorised users.
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
    // Multi-step digest conversation (intercepts free-text during a session)
    // -----------------------------------------------------------------------
    if let Some(state) = state_manager.sessions.remove(&user_id) {
        handle_digest_step(client, config, chat_id, user_id, text, state, state_manager).await;
        return;
    }

    // -----------------------------------------------------------------------
    // Top-level command dispatch
    // -----------------------------------------------------------------------
    match parse_command(text) {
        Some(Command::Status) => {
            tracing::info!("Handling /status command for user {}", user_id);
            handle_status(client, config, chat_id).await;
        }

        Some(Command::Digest) => {
            tracing::info!("Handling /digest command for user {}", user_id);
            state_manager.sessions.insert(user_id, ConversationState::new());
            let prompt = "🛠 <b>Digest Mode</b>\n\
                You can override individual inputs for this run. Type <code>skip</code> to keep the default for any step.\n\n\
                ✍️ <b>Step 1/3 – Context override</b>\n\
                Provide custom context text (or <code>skip</code>):";
            send_message(client, &tconfig.bot_token, chat_id, prompt, &tconfig.parse_mode).await;
        }

        Some(Command::Unknown(_)) | None => {
            let help = "❓ <b>Unknown command.</b>\n\n\
                Available commands:\n\
                /status — Machine status + AI kernel log analysis\n\
                /digest — Generate custom digest (with optional overrides)";
            send_message(client, &tconfig.bot_token, chat_id, help, &tconfig.parse_mode).await;
        }
    }
}

// ---------------------------------------------------------------------------
// /status handler – machine metrics + Gemini kernel log analysis
// ---------------------------------------------------------------------------

/// Collects system metrics and asks Gemini to analyse recent kernel log entries,
/// then sends both as two separate Telegram messages.
async fn handle_status(client: &Client, config: &Config, chat_id: i64) {
    let tconfig = config.telegram.as_ref().unwrap();

    // 1. Collect system metrics (this also fetches kernel logs).
    tracing::info!("Collecting system status...");
    let status = get_system_status();

    // 2. Send the metrics snapshot immediately so the user gets fast feedback.
    send_message(client, &tconfig.bot_token, chat_id, &format_system_status(&status), &tconfig.parse_mode).await;

    // 3. If we have log entries, ask Gemini to analyse them.
    if status.kernel_log_entries.is_empty() {
        send_message(client, &tconfig.bot_token, chat_id,
            "ℹ️ No kernel/journal warning entries found.", &tconfig.parse_mode).await;
        return;
    }

    send_message(client, &tconfig.bot_token, chat_id,
        "🔍 <b>Analysing kernel logs with Gemini…</b>", &tconfig.parse_mode).await;

    let now = Utc::now();
    let log_block = status.kernel_log_entries.join("\n");
    let prompt = format!(
        "You are a Linux system reliability expert.\n\
         The current date and time is: {}\n\n\
         The following are the last 20 warning-or-higher messages from journalctl on this machine:\n\
         <logs>\n{}\n</logs>\n\n\
         Instructions:\n\
         - Analyse each log entry. Consider its timestamp relative to the current time.\n\
         - Entries older than 7 days should be flagged as 'likely resolved' unless they recur.\n\
         - Identify any entries that are still relevant today (recent or recurring).\n\
         - Group findings: (1) Critical/Active issues, (2) Warnings worth monitoring, (3) Old/resolved entries.\n\
         - Be concise. Use plain text without markdown code blocks.\n\
         - Keep the total response under 1500 characters.\n\
         - Use emojis to indicate severity: 🔴 critical, 🟡 warning, 🟢 resolved/old.",
        now.format("%Y-%m-%d %H:%M UTC"),
        log_block
    );

    match gemini::call_gemini_text_for_bot(config, &prompt).await {
        Ok(analysis) => {
            let msg = format!(
                "🧠 <b>Gemini Kernel Log Analysis</b>\n\n{}",
                escape_html(&analysis)
            );
            send_message(client, &tconfig.bot_token, chat_id, &msg, &tconfig.parse_mode).await;
        }
        Err(e) => {
            tracing::error!("Gemini kernel log analysis failed: {}", e);
            send_message(client, &tconfig.bot_token, chat_id,
                &format!("⚠️ Gemini analysis unavailable: {}", escape_html(&e.to_string())),
                &tconfig.parse_mode).await;
        }
    }
}

// ---------------------------------------------------------------------------
// /digest – multi-step state machine
// ---------------------------------------------------------------------------

/// Advances the custom digest conversation by one step.
async fn handle_digest_step(
    client: &Client,
    config: &Config,
    chat_id: i64,
    user_id: u64,
    text: &str,
    mut state: ConversationState,
    state_manager: &mut StateManager,
) {
    let tconfig = config.telegram.as_ref().unwrap();
    // "skip" means keep the application default for this field.
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

            // Summarise overrides before running.
            let mut desc = String::from("📋 <b>Running digest with:</b>\n");
            desc.push_str(&format!("• Context: {}\n", if state.overrides.context.is_some() { "✅ custom" } else { "default" }));
            desc.push_str(&format!("• History: {}\n", if state.overrides.history.is_some() { "✅ custom" } else { "default" }));
            desc.push_str(&format!("• Lookback: {}\n", state.overrides.lookback_hours
                .map_or("default".to_string(), |h| format!("✅ {}h", h))));
            desc.push_str("\n⚙️ <b>Generating digest…</b> This may take a moment.");
            send_message(client, &tconfig.bot_token, chat_id, &desc, &tconfig.parse_mode).await;

            // Apply lookback override.
            let mut custom_config = config.clone();
            if let Some(h) = state.overrides.lookback_hours {
                custom_config.mattermost.lookback_hours = h;
            }

            match run_custom_digest(&custom_config, state.overrides).await {
                Ok(summary) => {
                    let msg = format!(
                        "✅ <b>Digest Summary</b>\n\n{}",
                        escape_html(&summary)
                    );
                    send_message(client, &tconfig.bot_token, chat_id, &msg, &tconfig.parse_mode).await;
                }
                Err(e) => {
                    tracing::error!("Custom digest failed: {}", e);
                    send_message(client, &tconfig.bot_token, chat_id,
                        &format_error(&e.to_string()), &tconfig.parse_mode).await;
                }
            }
        }

        CustomDigestStep::ReadyToRun => {}
    }
}

// ---------------------------------------------------------------------------
// Digest runner
// ---------------------------------------------------------------------------

/// Runs the customised Mattermost digest pipeline and returns the raw AI summary text.
/// Does NOT send an email and does NOT overwrite `history.txt`.
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
