// MIT License
// Copyright (c) 2026 Cedric Gegout

use async_trait::async_trait;
use reqwest::{Client, ClientBuilder};
use std::time::Duration;

use crate::config::MattermostConfig;
use crate::error::AppError;
use crate::models::{Channel, PostList, User};

/// A trait defining the required interactions with the Mattermost API.
#[async_trait]
pub trait MattermostApi {
    /// Retrieves the profile of the currently authenticated user.
    async fn get_me(&self) -> Result<User, AppError>;
    /// Retrieves a list of all channels the authenticated user has joined.
    async fn get_my_channels(&self) -> Result<Vec<Channel>, AppError>;
    /// Fetches a paginated list of posts for a specific channel since a given timestamp.
    async fn get_channel_posts(&self, channel_id: &str, since: i64, page: u32, per_page: u32) -> Result<PostList, AppError>;
    /// Resolves a list of Mattermost user IDs into full user objects (for rendering display names).
    async fn get_users_by_ids(&self, user_ids: &[String]) -> Result<Vec<User>, AppError>;
}

/// The concrete HTTP client implementation for the Mattermost API.
pub struct MattermostClient {
    /// The underlying asynchronous HTTP client.
    client: Client,
    /// The base URL of the Mattermost server.
    base_url: String,
    /// The personal access token used for authentication.
    token: String,
}

impl MattermostClient {
    /// Constructs a new `MattermostClient` from the given configuration.
    pub fn new(config: &MattermostConfig) -> Result<Self, AppError> {
        let client = ClientBuilder::new()
            .timeout(Duration::from_secs(config.request_timeout_seconds))
            .build()
            .map_err(|e| AppError::Config(format!("Failed to build HTTP client: {}", e)))?;

        let base_url = config.base_url.trim_end_matches('/').to_string();

        Ok(Self {
            client,
            base_url,
            token: config.personal_token.clone(),
        })
    }

    /// Formats the Authorization header for HTTP requests.
    fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

#[async_trait]
impl MattermostApi for MattermostClient {
    async fn get_me(&self) -> Result<User, AppError> {
        let url = format!("{}/api/v4/users/me", self.base_url);
        let res = self.client.get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;
            
        if !res.status().is_success() {
            return Err(AppError::Mattermost(format!("Failed to get current user: {}", res.status())));
        }
        
        Ok(res.json::<User>().await?)
    }

    async fn get_my_channels(&self) -> Result<Vec<Channel>, AppError> {
        let url = format!("{}/api/v4/users/me/channels", self.base_url);
        let res = self.client.get(&url)
            .header("Authorization", self.auth_header())
            .send()
            .await?;
            
        if !res.status().is_success() {
            return Err(AppError::Mattermost(format!("Failed to get my channels: {}", res.status())));
        }
        
        Ok(res.json::<Vec<Channel>>().await?)
    }

    async fn get_channel_posts(&self, channel_id: &str, since: i64, page: u32, per_page: u32) -> Result<PostList, AppError> {
        let url = format!("{}/api/v4/channels/{}/posts", self.base_url, channel_id);
        let res = self.client.get(&url)
            .header("Authorization", self.auth_header())
            .query(&[
                ("since", since.to_string()),
                ("page", page.to_string()),
                ("per_page", per_page.to_string()),
            ])
            .send()
            .await?;
            
        if !res.status().is_success() {
            return Err(AppError::Mattermost(format!("Failed to get posts for channel {}: {}", channel_id, res.status())));
        }
        
        Ok(res.json::<PostList>().await?)
    }

    async fn get_users_by_ids(&self, user_ids: &[String]) -> Result<Vec<User>, AppError> {
        if user_ids.is_empty() {
            return Ok(vec![]);
        }
        
        let url = format!("{}/api/v4/users/ids", self.base_url);
        let res = self.client.post(&url)
            .header("Authorization", self.auth_header())
            .json(user_ids)
            .send()
            .await?;
            
        if !res.status().is_success() {
            return Err(AppError::Mattermost(format!("Failed to resolve users: {}", res.status())));
        }
        
        Ok(res.json::<Vec<User>>().await?)
    }
}
