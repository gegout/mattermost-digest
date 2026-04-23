// MIT License
// Copyright (c) 2026 Cedric Gegout

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct User {
    pub id: String,
    pub username: String,
    pub first_name: String,
    pub last_name: String,
    pub email: String,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Channel {
    pub id: String,
    pub team_id: Option<String>,
    pub name: String,
    pub display_name: String,
    #[serde(rename = "type")]
    pub channel_type: String, // 'O' for public, 'P' for private, 'D' for direct, 'G' for group
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PostList {
    pub order: Vec<String>,
    pub posts: HashMap<String, Post>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Post {
    pub id: String,
    pub create_at: i64,
    pub update_at: i64,
    pub delete_at: i64,
    pub user_id: String,
    pub channel_id: String,
    pub message: String,
    // Add other fields if needed
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Team {
    pub id: String,
    pub name: String,
    pub display_name: String,
}
