// MIT License
// Copyright (c) 2026 Cedric Gegout

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents a user object returned by the Mattermost API.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct User {
    /// The unique identifier of the user.
    pub id: String,
    /// The username.
    pub username: String,
    /// The user's first name.
    pub first_name: String,
    /// The user's last name.
    pub last_name: String,
    /// The user's email address.
    pub email: String,
}

/// Represents a channel object returned by the Mattermost API.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Channel {
    /// The unique identifier of the channel.
    pub id: String,
    /// The ID of the team this channel belongs to, if applicable.
    pub team_id: Option<String>,
    /// The unique system name of the channel.
    pub name: String,
    /// The UI-friendly display name of the channel.
    pub display_name: String,
    /// The type of channel ('O' for public, 'P' for private, 'D' for direct, 'G' for group).
    #[serde(rename = "type")]
    pub channel_type: String,
}

/// Represents a paginated response of posts from the Mattermost API.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct PostList {
    /// Ordered list of post IDs as returned by the server.
    pub order: Vec<String>,
    /// A map of post IDs to their actual `Post` objects.
    pub posts: HashMap<String, Post>,
}

/// Represents an individual chat post/message in Mattermost.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Post {
    /// The unique identifier of the post.
    pub id: String,
    /// The timestamp (in milliseconds) when the post was created.
    pub create_at: i64,
    /// The timestamp (in milliseconds) when the post was last updated.
    pub update_at: i64,
    /// The timestamp (in milliseconds) when the post was deleted (0 if active).
    pub delete_at: i64,
    /// The ID of the user who authored the post.
    pub user_id: String,
    /// The ID of the channel where this post lives.
    pub channel_id: String,
    /// The raw text message content of the post.
    pub message: String,
}

/// Represents a team object returned by the Mattermost API.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Team {
    /// The unique identifier of the team.
    pub id: String,
    /// The system name of the team.
    pub name: String,
    /// The UI-friendly display name.
    pub display_name: String,
}
