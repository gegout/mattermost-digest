// MIT License
// Copyright (c) 2026 Cedric Gegout

use chrono::{DateTime, Duration, TimeZone, Utc};
use indicatif::{ProgressBar, ProgressStyle};
use std::collections::{HashMap, HashSet};

use crate::config::Config;
use crate::error::AppError;
use crate::mattermost::MattermostApi;
use crate::models::{Channel, Post, User};

pub struct DigestResult {
    pub markdown: String,
    pub has_messages: bool,
}

pub async fn generate_digest<M: MattermostApi>(
    client: &M,
    config: &Config,
    now: DateTime<Utc>,
) -> Result<DigestResult, AppError> {
    tracing::info!(">>>>>>>>>>>>>>>>>>>Starting digest generation<<<<<<<<<<<<<<<<<<");
    let since = now - Duration::hours(config.mattermost.lookback_hours as i64);
    let since_ms = since.timestamp_millis();

    tracing::info!("Authenticating with Mattermost and fetching user info...");
    let _me = client.get_me().await?;
    tracing::info!("Fetching all joined channels...");
    let channels = client.get_my_channels().await?;
    tracing::info!(
        "Found {} channels. Fetching messages from the past {} hours...",
        channels.len(),
        config.mattermost.lookback_hours
    );

    let mut total_messages = 0;
    let mut active_channels = 0;
    let mut channel_messages: Vec<(Channel, Vec<Post>)> = Vec::new();
    let mut user_ids_to_resolve = HashSet::new();

    let pb = ProgressBar::new(channels.len() as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({eta}) - {msg}")
        .unwrap()
        .progress_chars("#>-"));

    for channel in channels {
        pb.set_message(format!("Fetching {}", channel.display_name));
        tracing::debug!("Fetching posts for channel: {}", channel.display_name);
        let mut posts = Vec::new();
        let mut page = 0;

        // Loop to fetch paginated posts from the channel until we hit the time boundary
        loop {
            let list = client
                .get_channel_posts(&channel.id, since_ms, page, config.mattermost.per_page)
                .await?;
            let mut page_posts: Vec<Post> = list
                .posts
                .into_values()
                .filter(|p| p.delete_at == 0 && p.create_at >= since_ms)
                .collect();

            // Sort to ensure we count properly and stop if needed
            page_posts.sort_by_key(|p| p.create_at);

            let count = page_posts.len();
            for p in page_posts {
                user_ids_to_resolve.insert(p.user_id.clone());
                posts.push(p);
            }

            if count == 0 || (list.order.len() as u32) < config.mattermost.per_page {
                break;
            }
            page += 1;
        }

        // Ensure posts are ordered chronologically for the digest output
        posts.sort_by_key(|p| p.create_at);

        // Truncate to max_posts_per_channel if needed to prevent excessively huge digests
        if posts.len() > config.output.max_posts_per_channel as usize {
            posts = posts
                .into_iter()
                .take(config.output.max_posts_per_channel as usize)
                .collect();
        }

        if !posts.is_empty() {
            active_channels += 1;
            total_messages += posts.len();
        }

        if !posts.is_empty() || config.output.include_empty_channels {
            channel_messages.push((channel, posts));
        }
        pb.inc(1);
    }

    pb.finish_with_message("Done fetching messages");

    // Resolve user IDs to real usernames by querying Mattermost in bulk
    let user_ids: Vec<String> = user_ids_to_resolve.into_iter().collect();
    tracing::info!(
        "Resolving {} unique user IDs to usernames...",
        user_ids.len()
    );
    let users = client.get_users_by_ids(&user_ids).await.unwrap_or_default();

    // Create a fast lookup map for ID -> User
    let mut user_map = HashMap::new();
    for u in users {
        user_map.insert(u.id.clone(), u);
    }

    tracing::info!("Generating final markdown digest from fetched data...");
    let markdown = build_markdown(
        now,
        config.mattermost.lookback_hours,
        channel_messages.len(),
        active_channels,
        total_messages,
        channel_messages,
        &user_map,
    );

    Ok(DigestResult {
        markdown,
        has_messages: total_messages > 0,
    })
}

fn build_markdown(
    generated_at: DateTime<Utc>,
    lookback_hours: u32,
    total_channels_scanned: usize,
    active_channels: usize,
    total_messages: usize,
    channel_messages: Vec<(Channel, Vec<Post>)>,
    user_map: &HashMap<String, User>,
) -> String {
    let mut md = String::new();

    md.push_str("# Mattermost Digest\n");
    md.push_str(&format!("**Generated:** {}\n", generated_at.to_rfc2822()));
    md.push_str(&format!("**Window:** last {} hours\n\n", lookback_hours));

    md.push_str("## Summary\n");
    md.push_str(&format!("- Channels scanned: {}\n", total_channels_scanned));
    md.push_str(&format!("- Channels with activity: {}\n", active_channels));
    md.push_str(&format!("- Messages included: {}\n\n", total_messages));

    if total_messages == 0 {
        md.push_str("*No new messages in the configured time window.*\n");
        return md;
    }

    // Group by channel
    for (channel, posts) in channel_messages {
        if posts.is_empty() {
            md.push_str(&format!(
                "## {}\n*No new messages*\n\n",
                channel.display_name
            ));
            continue;
        }

        md.push_str(&format!("## {}\n", channel.display_name));

        let mut current_day = String::new();

        for post in posts {
            let dt = Utc.timestamp_millis_opt(post.create_at).unwrap();
            let day = dt.format("%Y-%m-%d").to_string();
            let time = dt.format("%H:%M").to_string();

            if day != current_day {
                md.push_str(&format!("### {}\n", day));
                current_day = day;
            }

            let author_name = user_map
                .get(&post.user_id)
                .map(|u| u.username.as_str())
                .unwrap_or("unknown_user");

            // very simple escaping of newlines for list format
            let cleaned_msg = post.message.replace('\n', "\n  ");

            md.push_str(&format!(
                "- **{}** — {}: {}\n",
                time, author_name, cleaned_msg
            ));
        }
        md.push('\n');
    }

    md
}
