# Mattermost Digest

`mattermost-digest` is a production-quality Rust CLI application that helps you reclaim your time from endless chat logs. It connects to your Mattermost server, fetches messages from your channels over a configurable time window (e.g., the last 24 hours), intelligently summarizes them using the Gemini AI, and sends a beautifully formatted HTML report directly to your email via the Gmail API.

## Motivation

In highly active remote teams, returning from a day off or just waking up to hundreds of unread Mattermost messages across dozens of channels can be overwhelming. Reading through everything takes too much time, but ignoring it risks missing critical information. 

**The goal of this application is to:**
1. **Save Time:** Provide an AI-powered "Executive Summary" at the top of your email that highlights exactly what you need to know, grouped into what is important to you, what is important for the team, and what is just FYI.
2. **Be Unobtrusive:** Operate completely transparently. The application strictly relies on read-only endpoints and intentionally does **not** change any channel's "unread" or "viewed" status. You can read the AI digest and still have your original unread badges intact in your Mattermost client if you want to respond later.
3. **Be Beautiful:** Deliver a polished, styled HTML email directly to your inbox that is easy to read on both desktop and mobile.

---

## Features
- 🚀 **Fast & Concurrent:** Written in Rust for maximum performance and low memory footprint.
- 🤖 **AI Summarization:** Uses Google Gemini to generate a smart, prioritized executive summary.
- 📧 **Gmail Integration:** Sends emails from your own account using OAuth 2.0 (Installed-App flow).
- 🎨 **HTML Formatting:** Converts Markdown chat logs into a highly readable, styled HTML newsletter.
- 📊 **Visual Feedback:** Displays a real-time progress bar while fetching messages.
- ⚙️ **Flexible CLI:** Easily override default configurations directly from the command line.

---

## Prerequisites
- **Rust toolchain** (1.70+ recommended).
- A **Mattermost Personal Access Token**.
- A **Google Cloud Project** with the Gmail API enabled and an OAuth Installed-App client secret.
- A **Google Gemini API Key**.

## Google OAuth Setup
To send emails via the Gmail API, you need Google OAuth credentials:
1. Go to the [Google Cloud Console](https://console.cloud.google.com/).
2. Create a new project or select an existing one.
3. Enable the **Gmail API** for the project.
4. Go to **OAuth consent screen** and configure it (you can set it to "External" and add your own email as a Test User).
5. Go to **Credentials**, click **Create Credentials** -> **OAuth client ID**.
6. Select **Desktop app** (Installed App) as the application type.
7. Download the `client_secret.json` file.
8. Place the `client_secret.json` file in `~/.config/mattermost-digest/` (or any path you prefer, and configure it in `config.toml`).

## Mattermost Setup
To fetch messages from Mattermost, you need a Personal Access Token:
1. Log into your Mattermost server.
2. Go to **Profile** > **Security** > **Personal Access Tokens**.
3. Create a new token and save it securely.

## Configuration
The application reads its configuration from `~/.config/mattermost-digest/config.toml`. 

See the provided `config.example.toml` for the layout. Copy it into the correct directory:
```bash
mkdir -p ~/.config/mattermost-digest
cp config.example.toml ~/.config/mattermost-digest/config.toml
```

Populate it with your Mattermost token, your Gmail secret path, your emails, and your Gemini API key.

## Build Instructions
Build the highly-optimized production version using standard Cargo commands:
```bash
cargo build --release
```
The executable will be located at `target/release/mattermost-digest`. You can copy it to your local bin folder to run it from anywhere:
```bash
cp target/release/mattermost-digest ~/.local/bin/
```

---

## Run Instructions & CLI Usage

The application features an intuitive CLI to manage authentication, test connections, and execute the digest pipeline.

### 1. Test Connections
Before running the full pipeline, verify that all external services are configured correctly:
```bash
mattermost-digest test mattermost
mattermost-digest test gmail
mattermost-digest test gemini
```

### 2. Authenticate Gmail
```bash
mattermost-digest auth gmail
```
This will open your default web browser. Follow the prompts to authenticate with your Google account. It will securely store the OAuth token cache in `~/.config/mattermost-digest/tokencache.json`. Future runs will use this cached token silently.

### 3. Dry-Run Digest
```bash
mattermost-digest run --dry-run
```
This will fetch all new messages and generate the markdown and HTML digest, but it will **exit without sending the email**. This is great for testing your configuration locally.

### 4. Run the Full Pipeline
```bash
mattermost-digest run
```
This executes the entire workflow:
1. Fetches all channels and messages from the last 24 hours (or configured window).
2. Sends the raw logs to Gemini for intelligent summarization.
3. Compiles the AI summary and raw logs into a styled HTML document.
4. Uses Gmail OAuth to email the report to your configured inbox.

### 5. Override Configuration on the Fly
You can temporarily override settings in your `config.toml` directly from the CLI:
```bash
mattermost-digest run --lookback-hours 12 --my-username "cgegout" --max-posts-per-channel 100
```
Run `mattermost-digest run --help` to see all available override options.

---

## Security Notes
- **Never commit your `config.toml`**, `client_secret.json`, or `tokencache.json` to version control.
- Restrict permissions on your config files (e.g., `chmod 600 ~/.config/mattermost-digest/config.toml`).
- Use tokens with the minimal required permissions on Google, Gemini, and Mattermost.

## Mattermost REST APIs Used
The application intentionally uses only a strictly read-only subset of the Mattermost API:
- `GET /api/v4/users/me` (To validate the token)
- `GET /api/v4/users/me/channels` (To discover channels)
- `GET /api/v4/channels/{channel_id}/posts?since={unix_ms}&page={page}&per_page={per_page}` (To fetch recent posts)
- `POST /api/v4/users/ids` (To resolve author user IDs)

**To satisfy the strict constraint that the tool must not mark any messages as read or viewed**, view-marking endpoints under channel views and unread-state retrievals are intentionally **never called**.
