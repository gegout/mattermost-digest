// MIT License
// Copyright (c) 2026 Cedric Gegout

use async_trait::async_trait;
use reqwest::{Client, ClientBuilder};
use std::time::Duration;

use crate::config::MattermostConfig;
use crate::error::AppError;
use crate::models::{Channel, PostList, User};

#[async_trait]
pub trait MattermostApi {
    async fn get_me(&self) -> Result<User, AppError>;
    async fn get_my_channels(&self) -> Result<Vec<Channel>, AppError>;
    async fn get_channel_posts(&self, channel_id: &str, since: i64, page: u32, per_page: u32) -> Result<PostList, AppError>;
    async fn get_users_by_ids(&self, user_ids: &[String]) -> Result<Vec<User>, AppError>;
}

pub struct MattermostClient {
    client: Client,
    base_url: String,
    token: String,
}

impl MattermostClient {
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
