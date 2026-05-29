// MIT License
// Copyright (c) 2026 Cedric Gegout

use chrono::Utc;
use reqwest::Client;
use serde_json::{json, Value};
use tokio::time::{sleep, Duration};

use crate::config::Config;
use crate::digest::{self, ChannelProgress};
use crate::error::AppError;
use crate::gemini;
use crate::mattermost::MattermostClient;
use crate::system_status::get_system_status;
use crate::telegram_commands::{
    parse_command, Command, ConversationState, CustomDigestStep, DigestOverrides, StateManager,
};
use crate::telegram_format::{escape_html, format_error, format_system_status};
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Pipeline stage definitions
// ---------------------------------------------------------------------------

/// Labels for each stage of the digest pipeline, in order.
const STAGES: &[&str] = &[
    "Connecting to Mattermost",
    "Fetching messages from channels",
    "Building digest",
    "Summarising with Gemini",
];

/// Builds the full pipeline progress message.
/// `stage` is the 0-based currently-active stage index.
/// `sub` is an optional sub-progress bar shown only during the fetch stage.
fn progress_text(stage: usize, sub: Option<&ChannelProgress>) -> String {
    let total = STAGES.len();
    let filled = (stage * 10) / total;
    let empty = 10 - filled;
    let bar = format!(
        "[{}{}] {}/{}",
        "█".repeat(filled),
        "░".repeat(empty),
        stage,
        total
    );

    let mut msg = format!("⚙️ <b>Generating Digest…</b>\n\n<code>{}</code>\n\n", bar);
    for (i, label) in STAGES.iter().enumerate() {
        let icon = if i < stage {
            "✅"
        } else if i == stage {
            "🔄"
        } else {
            "⏳"
        };
        msg.push_str(&format!("{} {}\n", icon, label));

        // Inline the channel sub-bar right after the fetch stage label.
        if i == 1 && i == stage {
            if let Some(cp) = sub {
                let ch_filled = (cp.current * 10) / cp.total.max(1);
                let ch_empty = 10 - ch_filled;
                let ch_bar = format!(
                    "   <code>[{}{}] {}/{}</code>",
                    "▪".repeat(ch_filled),
                    "▫".repeat(ch_empty),
                    cp.current,
                    cp.total,
                );
                msg.push_str(&format!("{}\n", ch_bar));
            }
        }
    }
    msg
}

/// Builds the final "done" progress message text.
fn progress_done_text() -> String {
    let total = STAGES.len();
    let bar = format!("[{}] {}/{}", "█".repeat(10), total, total);
    let mut msg = format!("✅ <b>Digest Complete</b>\n\n<code>{}</code>\n\n", bar);
    for label in STAGES.iter() {
        msg.push_str(&format!("✅ {}\n", label));
    }
    msg
}

// ---------------------------------------------------------------------------
// Telegram API helpers
// ---------------------------------------------------------------------------

/// Sends a Telegram HTML message and returns the `message_id` of the sent message.
/// Returns `None` on network or parse error.
async fn send_message_get_id(
    client: &Client,
    token: &str,
    chat_id: i64,
    text: &str,
    parse_mode: &str,
) -> Option<i64> {
    let url = format!("https://api.telegram.org/bot{}/sendMessage", token);
    let truncated: String = text.chars().take(4090).collect();
    let payload = json!({
        "chat_id": chat_id,
        "text": truncated,
        "parse_mode": parse_mode,
    });
    match client.post(&url).json(&payload).send().await {
        Ok(resp) => resp
            .json::<Value>()
            .await
            .ok()
            .and_then(|v| v.get("result")?.get("message_id")?.as_i64()),
        Err(e) => {
            tracing::error!("send_message_get_id failed: {}", e);
            None
        }
    }
}

/// Sends a Telegram HTML message to a specific chat.
/// Long messages are automatically truncated to Telegram's 4 096-character limit.
async fn send_message(client: &Client, token: &str, chat_id: i64, text: &str, parse_mode: &str) {
    send_message_get_id(client, token, chat_id, text, parse_mode).await;
}

/// Edits an existing Telegram message in-place.
async fn edit_message(
    client: &Client,
    token: &str,
    chat_id: i64,
    message_id: i64,
    text: &str,
    parse_mode: &str,
) {
    let url = format!("https://api.telegram.org/bot{}/editMessageText", token);
    let truncated: String = text.chars().take(4090).collect();
    let payload = json!({
        "chat_id": chat_id,
        "message_id": message_id,
        "text": truncated,
        "parse_mode": parse_mode,
    });
    if let Err(e) = client.post(&url).json(&payload).send().await {
        tracing::warn!("Failed to edit message {}: {}", message_id, e);
    }
}

// ---------------------------------------------------------------------------
// Progress reporter
// ---------------------------------------------------------------------------

/// Owns the context required to update a live Telegram progress message.
#[derive(Clone)]
struct DigestProgress {
    client: Client,
    token: String,
    chat_id: i64,
    message_id: Option<i64>,
    parse_mode: String,
}

impl DigestProgress {
    /// Updates the live progress message to reflect the given pipeline `stage`.
    async fn advance(&self, stage: usize) {
        if let Some(mid) = self.message_id {
            tracing::info!("Digest progress: stage {}/{}", stage, STAGES.len());
            edit_message(
                &self.client,
                &self.token,
                self.chat_id,
                mid,
                &progress_text(stage, None),
                &self.parse_mode,
            )
            .await;
        }
    }

    /// Updates the live progress message to the final "done" state.
    async fn complete(&self) {
        if let Some(mid) = self.message_id {
            edit_message(
                &self.client,
                &self.token,
                self.chat_id,
                mid,
                &progress_done_text(),
                &self.parse_mode,
            )
            .await;
        }
    }

    /// Appends a status message to the current stage description.
    async fn status(&self, stage: usize, status: &str) {
        if let Some(mid) = self.message_id {
            let mut text = progress_text(stage, None);
            text.push_str(&format!("\n\n<i>{}</i>", status));
            edit_message(
                &self.client,
                &self.token,
                self.chat_id,
                mid,
                &text,
                &self.parse_mode,
            )
            .await;
        }
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

    // Multi-step digest conversation (intercepts free-text during a session).
    if let Some(state) = state_manager.sessions.remove(&user_id) {
        handle_digest_step(client, config, chat_id, user_id, text, state, state_manager).await;
        return;
    }

    // Top-level command dispatch.
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

async fn handle_status(client: &Client, config: &Config, chat_id: i64) {
    let tconfig = config.telegram.as_ref().unwrap();

    // Send an immediate loading message so the user knows we are working.
    let status_msg_id = send_message_get_id(
        client, &tconfig.bot_token, chat_id,
        "⏳ <b>Collecting system metrics…</b>", &tconfig.parse_mode
    ).await;

    tracing::info!("Collecting system status...");
    let status = get_system_status();

    // Update the loading message with the rich metrics snapshot.
    if let Some(mid) = status_msg_id {
        edit_message(
            client, &tconfig.bot_token, chat_id, mid,
            &format_system_status(&status), &tconfig.parse_mode
        ).await;
    } else {
        send_message(client, &tconfig.bot_token, chat_id, &format_system_status(&status), &tconfig.parse_mode).await;
    }

    // If there are no log signals and the system is Healthy, skip Gemini entirely.
    if status.log_signals.is_empty() && status.health == crate::system_status::HealthStatus::Healthy {
        tracing::info!("System is healthy with no log signals — skipping Gemini analysis.");
        return;
    }

    // Otherwise, ask Gemini to comment on the health picture and any log signals.
    send_message(client, &tconfig.bot_token, chat_id,
        "🧠 <b>Analysing system health with Gemini…</b>", &tconfig.parse_mode).await;

    let now = Utc::now();
    let log_block = if status.log_signals.is_empty() {
        "No log signals collected.".to_string()
    } else {
        status.log_signals.join("\n")
    };

    let prompt = format!(
        "You are a Linux system reliability expert.\n\
         Current date/time: {}\n\n\
         Machine snapshot:\n\
         - CPU: {:.1}%  Load: {:.2} / {:.2} / {:.2}  ({} CPUs)\n\
         - Memory: {} MB / {} MB used\n\
         - Disk /: {} GB / {} GB used\n\
         - Uptime: {}s\n\
         - Overall health assessment: {}\n\
         - Findings: {}\n\n\
         Log signals (best-effort, may be empty):\n<logs>\n{}\n</logs>\n\n\
         Instructions:\n\
         - Review the machine snapshot and any log signals.\n\
         - Flag entries older than 7 days as likely resolved unless recurring.\n\
         - Group findings: (1) Active/Critical, (2) Worth monitoring, (3) Resolved/old.\n\
         - After the analysis, provide exactly 3 Recommended Approaches to improve machine health or stability.\n\
         - Use ONLY Telegram-compatible HTML tags for formatting (<b>, <i>, <code>).\n\
         - DO NOT use markdown symbols like **, *, or ###.\n\
         - Keep the total response under 1500 characters.\n\
         - Use emojis: 🔴 critical, 🟡 warning, 🟢 resolved/healthy.",
        now.format("%Y-%m-%d %H:%M UTC"),
        status.cpu_usage, status.load_1m, status.load_5m, status.load_15m, status.cpu_count,
        status.memory_used_mb, status.memory_total_mb,
        status.disk_used_gb, status.disk_total_gb,
        status.uptime_seconds,
        status.health.label(),
        status.findings.join("; "),
        log_block,
    );

    match gemini::call_gemini_text_for_bot(config, &prompt).await {
        Ok(analysis) => {
            // Note: We don't escape_html here because we asked Gemini to provide valid HTML tags.
            let msg = format!("🧠 <b>Gemini Health Analysis</b>\n\n{}", analysis);
            send_message(client, &tconfig.bot_token, chat_id, &msg, &tconfig.parse_mode).await;
        }
        Err(e) => {
            tracing::error!("Gemini health analysis failed: {}", e);
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
                        let msg = "❌ Expected an integer for lookback hours. Try again (or type <code>skip</code>):";
                        send_message(client, &tconfig.bot_token, chat_id, msg, &tconfig.parse_mode).await;
                        state.step = CustomDigestStep::AskLookback;
                        state_manager.sessions.insert(user_id, state);
                        return;
                    }
                }
            }

            // Summarise what will be overridden.
            let mut desc = String::from("📋 <b>Running digest with:</b>\n");
            desc.push_str(&format!("• Context: {}\n", if state.overrides.context.is_some() { "✅ custom" } else { "default" }));
            desc.push_str(&format!("• History: {}\n", if state.overrides.history.is_some() { "✅ custom" } else { "default" }));
            desc.push_str(&format!("• Lookback: {}\n\n", state.overrides.lookback_hours
                .map_or("default".to_string(), |h| format!("✅ {}h", h))));
            // Append the initial progress bar (stage 0) to the same message.
            desc.push_str(&progress_text(0, None));

            // Send the combined config summary + initial progress bar, capture the message_id.
            let progress_msg_id = send_message_get_id(
                client, &tconfig.bot_token, chat_id, &desc, &tconfig.parse_mode
            ).await;

            // Build the progress reporter.
            let progress = DigestProgress {
                client: client.clone(),
                token: tconfig.bot_token.clone(),
                chat_id,
                message_id: progress_msg_id,
                parse_mode: tconfig.parse_mode.clone(),
            };

            // Apply lookback override.
            let mut custom_config = config.clone();
            if let Some(h) = state.overrides.lookback_hours {
                custom_config.mattermost.lookback_hours = h;
            }

            match run_custom_digest(&custom_config, state.overrides, &progress).await {
                Ok(summary) => {
                    progress.complete().await;
                    // Note: We don't escape_html here because we asked Gemini for valid HTML.
                    let msg = format!("📝 <b>Digest Summary</b>\n\n{}", summary);
                    send_message(client, &tconfig.bot_token, chat_id, &msg, &tconfig.parse_mode).await;
                }
                Err(e) => {
                    tracing::error!("Custom digest failed: {}", e);
                    // Replace the progress bar with an error.
                    if let Some(mid) = progress.message_id {
                        edit_message(client, &tconfig.bot_token, chat_id, mid,
                            &format_error(&e.to_string()), &tconfig.parse_mode).await;
                    } else {
                        send_message(client, &tconfig.bot_token, chat_id,
                            &format_error(&e.to_string()), &tconfig.parse_mode).await;
                    }
                }
            }
        }

        CustomDigestStep::ReadyToRun => {}
    }
}

// ---------------------------------------------------------------------------
// Digest runner
// ---------------------------------------------------------------------------

/// Runs the customised Mattermost digest pipeline, reporting progress at each stage.
/// Does NOT send an email and does NOT overwrite `history.txt`.
async fn run_custom_digest(
    config: &Config,
    overrides: DigestOverrides,
    progress: &DigestProgress,
) -> Result<String, AppError> {
    tracing::info!("Custom digest triggered from Telegram bot.");

    // Stage 0 → 1: connecting to Mattermost.
    progress.advance(0).await;
    let mm_client = MattermostClient::new(&config.mattermost)?;
    let now = Utc::now();

    // Stage 1: fetching messages — spawn a task that listens for per-channel
    // progress and edits the Telegram message with a sub-bar, rate-limited
    // to at most one edit per second to stay within Telegram API limits.
    progress.advance(1).await;

    let (channel_tx, mut channel_rx) = mpsc::channel::<ChannelProgress>(64);

    // Clone everything the listener task needs.
    let listener_client  = progress.client.clone();
    let listener_token   = progress.token.clone();
    let listener_chat_id = progress.chat_id;
    let listener_msg_id  = progress.message_id;
    let listener_mode    = progress.parse_mode.clone();

    let listener = tokio::spawn(async move {
        use std::time::Instant;
        let mut last_edit = Instant::now() - std::time::Duration::from_secs(2);
        while let Some(cp) = channel_rx.recv().await {
            tracing::debug!(
                "Channel progress: {}/{} – {}",
                cp.current, cp.total, cp.channel_name
            );
            // Rate-limit: edit at most once per second.
            if last_edit.elapsed() >= std::time::Duration::from_millis(1000)
                || cp.current == cp.total
            {
                if let Some(mid) = listener_msg_id {
                    let text = progress_text(1, Some(&cp));
                    edit_message(
                        &listener_client,
                        &listener_token,
                        listener_chat_id,
                        mid,
                        &text,
                        &listener_mode,
                    )
                    .await;
                    last_edit = Instant::now();
                }
            }
        }
    });

    // Run the digest; the channel sender is dropped when generate_digest returns,
    // which signals the listener task to finish.
    let result = digest::generate_digest(&mm_client, config, now, Some(channel_tx)).await?;

    // Wait for the listener to finish its last edit before we advance stages.
    let _ = listener.await;

    // Stage 2: digest built, handing to Gemini.
    progress.advance(2).await;

    // Stage 3: summarising with Gemini.
    progress.advance(3).await;

    let (gemini_tx, mut gemini_rx) = mpsc::channel::<String>(10);
    let gemini_listener_progress = progress.clone();
    let gemini_listener = tokio::spawn(async move {
        while let Some(status) = gemini_rx.recv().await {
            gemini_listener_progress.status(3, &status).await;
        }
    });

    let summary = gemini::summarize_custom_digest(
        config,
        &result.markdown,
        overrides.context,
        overrides.history,
        true, // use_html
        Some(gemini_tx),
    )
    .await?;

    let _ = gemini_listener.await;

    Ok(summary)
}
