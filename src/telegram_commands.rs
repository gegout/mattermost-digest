// MIT License
// Copyright (c) 2026 Cedric Gegout

use std::collections::HashMap;

/// Top-level commands recognised by the Telegram bot.
#[derive(Debug, Clone)]
pub enum Command {
    /// Returns the current machine resource status with AI log analysis.
    Status,
    /// Launches the interactive custom digest wizard (with optional overrides).
    Digest,
    /// Unknown / unrecognised command.
    Unknown(String),
}

/// Parses the first word of an incoming message into a `Command`.
/// Leading `/` is stripped automatically.
pub fn parse_command(text: &str) -> Option<Command> {
    let cmd_str = text.trim_start_matches('/');
    let first = cmd_str.split_whitespace().next()?;
    match first.to_lowercase().as_str() {
        "status" => Some(Command::Status),
        "digest" => Some(Command::Digest),
        word => Some(Command::Unknown(word.to_string())),
    }
}

/// Fields that the user can override during a custom digest session.
#[derive(Debug, Clone, Default)]
pub struct DigestOverrides {
    /// Replacement for the content of `context.txt`.
    pub context: Option<String>,
    /// Replacement for the content of `history.txt`.
    pub history: Option<String>,
    /// Override for `mattermost.lookback_hours`.
    pub lookback_hours: Option<u32>,
}

/// Tracks which step of the multi-step custom digest flow is active.
#[derive(Debug, Clone, PartialEq)]
pub enum CustomDigestStep {
    AskContext,
    AskHistory,
    AskLookback,
    ReadyToRun,
}

/// Full state for an in-progress custom digest conversation.
#[derive(Debug, Clone)]
pub struct ConversationState {
    pub step: CustomDigestStep,
    pub overrides: DigestOverrides,
}

impl ConversationState {
    /// Initialises a fresh conversation starting at the context override step.
    pub fn new() -> Self {
        Self {
            step: CustomDigestStep::AskContext,
            overrides: DigestOverrides::default(),
        }
    }
}

/// Holds all active multi-step sessions keyed by Telegram user ID.
pub struct StateManager {
    pub sessions: HashMap<u64, ConversationState>,
}

impl StateManager {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }
}
