// MIT License
// Copyright (c) 2026 Cedric Gegout

use std::io::Cursor;
use google_gmail1::api::{Label, Message, ModifyMessageRequest};
use google_gmail1::Gmail;
use yup_oauth2::{read_application_secret, InstalledFlowAuthenticator, InstalledFlowReturnMethod};
use google_gmail1::hyper_rustls;
use google_gmail1::hyper_util;

use crate::config::GmailConfig;
use crate::error::AppError;

pub async fn get_gmail_client(
    config: &GmailConfig,
) -> Result<Gmail<hyper_rustls::HttpsConnector<hyper_util::client::legacy::connect::HttpConnector>>, AppError> {
    let secret_path = crate::config::expand_tilde(&config.client_secret_path);
    if !secret_path.exists() {
        return Err(AppError::Gmail(format!(
            "Google Client Secret not found at {:?}",
            secret_path
        )));
    }

    let secret = read_application_secret(&secret_path)
        .await
        .map_err(|e| AppError::Gmail(format!("Failed to read client secret: {}", e)))?;

    let token_cache_path = crate::config::expand_tilde(&config.token_cache_path);

    let auth = InstalledFlowAuthenticator::builder(
        secret,
        InstalledFlowReturnMethod::HTTPRedirect,
    )
    .persist_tokens_to_disk(token_cache_path)
    .build()
    .await
    .map_err(|e| AppError::Gmail(format!("Failed to build authenticator: {}", e)))?;

    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_native_roots()
        .expect("no native root CA certificates found")
        .https_or_http()
        .enable_http1()
        .build();

    let client = hyper_util::client::legacy::Client::builder(
        hyper_util::rt::TokioExecutor::new()
    ).build(https);

    Ok(Gmail::new(client, auth))
}

pub async fn send_digest_email(
    config: &GmailConfig,
    html_content: &str,
) -> Result<(), AppError> {
    let hub = get_gmail_client(config).await?;

    let to = &config.to_email;
    let from = &config.from_email;
    let subject = &config.email_subject;

    let mime_message = format!(
        "To: {}\r\nFrom: {}\r\nSubject: {}\r\nContent-Type: text/html; charset=\"UTF-8\"\r\n\r\n{}",
        to, from, subject, html_content
    );

    let message = Message::default();

    tracing::info!("Uploading the compiled digest email to Gmail and sending to {}...", to);
    let (_, result) = hub
        .users()
        .messages_send(message, "me")
        .upload(Cursor::new(mime_message.into_bytes()), "message/rfc822".parse().unwrap())
        .await
        .map_err(|e| AppError::Gmail(format!("Failed to send email: {}", e)))?;

    tracing::info!("Digest email successfully sent!");

    if let Some(msg_id) = result.id {
        tracing::info!("Finding or creating 'Mattermost' label...");
        let mut label_id = None;
        
        // List existing labels
        if let Ok((_, labels_resp)) = hub.users().labels_list("me").doit().await {
            if let Some(labels) = labels_resp.labels {
                for label in labels {
                    if label.name.as_deref() == Some("Mattermost") {
                        label_id = label.id;
                        break;
                    }
                }
            }
        }

        // Create label if it doesn't exist
        if label_id.is_none() {
            tracing::info!("Label 'Mattermost' not found, creating it...");
            let new_label = Label {
                name: Some("Mattermost".to_string()),
                label_list_visibility: Some("labelShow".to_string()),
                message_list_visibility: Some("show".to_string()),
                ..Default::default()
            };
            if let Ok((_, created_label)) = hub.users().labels_create(new_label, "me").doit().await {
                label_id = created_label.id;
            }
        }

        // Apply the label to the sent message
        if let Some(l_id) = label_id {
            tracing::info!("Applying 'Mattermost' label to the sent message...");
            let modify_req = ModifyMessageRequest {
                add_label_ids: Some(vec![l_id]),
                ..Default::default()
            };
            let _ = hub.users().messages_modify(modify_req, "me", &msg_id).doit().await;
            tracing::info!("Label successfully applied!");
        } else {
            tracing::warn!("Failed to find or create the 'Mattermost' label.");
        }
    }

    Ok(())
}

pub async fn test_auth(config: &GmailConfig) -> Result<(), AppError> {
    let hub = get_gmail_client(config).await?;
    
    // Test the token by fetching profile
    // Note: get_profile does not support media upload, so it has a .doit() wrapper in v7.
    let _ = hub
        .users()
        .get_profile("me")
        .doit()
        .await
        .map_err(|e| AppError::Gmail(format!("Failed to get profile: {}", e)))?;
        
    Ok(())
}
