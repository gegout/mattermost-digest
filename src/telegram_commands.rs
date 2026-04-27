// MIT License
// Copyright (c) 2026 Cedric Gegout

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum Command {
    Status,
    Digest,
    CustomDigest,
    Cancel,
    Unknown(String),
}

pub fn parse_command(text: &str) -> Option<Command> {
    let mut cmd_str = text;
    if cmd_str.starts_with('/') {
        cmd_str = &cmd_str[1..];
    }
    
    let parts: Vec<&str> = cmd_str.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    
    match parts[0].to_lowercase().as_str() {
        "status" => Some(Command::Status),
        "digest" => Some(Command::Digest),
        "customdigest" | "custom" => Some(Command::CustomDigest),
        "cancel" => Some(Command::Cancel),
        cmd => Some(Command::Unknown(cmd.to_string())),
    }
}

#[derive(Debug, Clone, Default)]
pub struct DigestOverrides {
    pub context: Option<String>,
    pub history: Option<String>,
    pub lookback_hours: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum CustomDigestStep {
    AskContext,
    AskHistory,
    AskLookback,
    ReadyToRun,
}

#[derive(Debug, Clone)]
pub struct ConversationState {
    pub step: CustomDigestStep,
    pub overrides: DigestOverrides,
}

impl ConversationState {
    pub fn new() -> Self {
        Self {
            step: CustomDigestStep::AskContext,
            overrides: DigestOverrides::default(),
        }
    }
}

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
