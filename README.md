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
- 🧠 **Context & Continuity:** Supports a persistent requester profile (`context.txt`) and an auto-generated rolling memory (`history.txt`) so the AI remembers ongoing topics.
- 📧 **Gmail Integration:** Sends emails from your own account using OAuth 2.0 (Installed-App flow).
- 🎨 **HTML Formatting:** Converts Markdown chat logs into a highly readable, styled HTML newsletter.
- 📊 **Visual Feedback:** Real-time progress bars and sub-bars for channel-level granularity in the CLI and Telegram bot.
- 🤖 **Telegram Bot Mode:** Interact with the digest engine and monitor your machine's health via a secure Telegram interface.
- 🔄 **Dynamic Model Fallback:** Automatically discovers and switches to alternate Gemini models if the primary model is unavailable or unsupported.
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

---

## Advanced Context & Continuity
To make the AI summaries highly personalized and maintain continuity across digest runs, `mattermost-digest` supports two powerful text files located in your config directory (`~/.config/mattermost-digest/`):

### 1. `context.txt`
This file allows you to define who you are and what matters to you. The content of this file is injected directly into the Gemini prompt. 
- **Example content:**
  ```text
  I am Cedric, leading product management for the Antigravity tooling platform. 
  I prioritize platform stability, new infrastructure integration initiatives, and unblocking QA teams.
  I don't need to know about standard HR announcements, routine CI pipeline hiccups unless they break main, or lunch plans. Focus on engineering blockers, executive decisions, and active product feature regressions.
  ```

### 2. `history.txt`
The application maintains a "rolling history" to provide continuity. 
- **How it works:** During each run, the app reads `history.txt` (if it exists) to understand ongoing threads from the *previous* run. Before exiting, it automatically makes a lightweight background call to Gemini to generate a *new* compressed continuity readout of the *current* chat logs. This readout replaces `history.txt` on disk, ready for the next cycle.
- **Benefits:** Gemini remembers that a particular bug was actively being debugged yesterday, allowing it to highlight new developments accurately today instead of treating every digest like a fresh amnesiac session.

Both files are optional. If they are missing, the application will emit a soft warning but will continue to run gracefully.

---

## 🤖 Telegram Bot Interface

The application includes a fully interactive Telegram bot mode (`bot` command). This allows you to trigger digests and monitor your machine remotely.

### Commands
- `/status`: Generates a rich machine health report (CPU, RAM, Disk, Load, Uptime, Top Processes).
  - 🧠 Includes an **AI Health Analysis**: Gemini scans best-effort log signals (syslog, kern.log, crash logs) to identify active issues or warnings.
  - 🔒 **Non-privileged**: Designed to run as a normal user; fallback mechanisms handle restricted log access gracefully.
- `/digest`: Launches a multi-step interactive wizard to run a custom digest.
  - 🛠️ Allows overriding **Context**, **History**, and **Lookback Hours** for a single run.
  - 📈 Provides a **live progress bar** with per-channel updates.

### Bot Configuration
Ensure your `config.toml` contains the `[telegram]` section with your bot token and authorized user IDs:
```toml
[telegram]
bot_token = "your_bot_token"
allowed_user_ids = [123456789]  # Restrict access to your own IDs
```

## 🔄 Dynamic Gemini Fallback

To ensure maximum reliability, `mattermost-digest` features an advanced dynamic fallback mechanism for the Gemini AI. 

- **Initial Attempt:** The app always starts with your configured primary model.
- **Intelligent Retries:** If a model fails with a transient error (503 Unavailable, 429 Rate Limit), it retries up to 3 times with exponential backoff.
- **Dynamic Discovery:** If the primary model is exhausted, unsupported, or non-existent, the app calls the Gemini API to discover currently available models.
- **Smart Selection:** Discovered models are ranked based on their capabilities (must support `generateContent`) and tier (preferring `flash-lite`, `flash`, and `pro` variants).
- **Execution Limit:** The app will try at most 3 different models before returning a comprehensive failure report.

This mechanism ensures that your digest is delivered even if specific Gemini models are under heavy load or have been deprecated.

---

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

### 3. Run as a Telegram Bot
```bash
mattermost-digest bot
```
This starts the bot in long-polling mode. You can now chat with your bot on Telegram.

### 4. Dry-Run Digest (CLI)
```bash
mattermost-digest run --dry-run
```
This will fetch all new messages and generate the markdown and HTML digest, but it will **exit without sending the email**. This is great for testing your configuration locally.

### 5. Run the Full Pipeline (CLI)
```bash
mattermost-digest run
```
This executes the entire workflow:
1. Fetches all channels and messages from the last 24 hours (or configured window).
2. Reads your `context.txt` and `history.txt` (if available).
3. Sends the raw logs + context to Gemini for intelligent summarization.
4. Generates and saves a new `history.txt` for your next run.
5. Compiles the AI summary and raw logs into a styled HTML document.
6. Uses Gmail OAuth to email the report to your configured inbox.

### 6. Override Configuration on the Fly (CLI)
You can temporarily override settings in your `config.toml` directly from the CLI:
```bash
mattermost-digest run --lookback-hours 12 --my-username "your name in mattermost" --max-posts-per-channel 100
```
Run `mattermost-digest run --help` to see all available override options.

---

## Security Notes
- **Never commit your `config.toml`**, `client_secret.json`, `context.txt`, `history.txt`, or `tokencache.json` to version control.
- Restrict permissions on your config files (e.g., `chmod 600 ~/.config/mattermost-digest/config.toml`).
- Use tokens with the minimal required permissions on Google, Gemini, and Mattermost.

## Mattermost REST APIs Used
The application intentionally uses only a strictly read-only subset of the Mattermost API:
- `GET /api/v4/users/me` (To validate the token)
- `GET /api/v4/users/me/channels` (To discover channels)
- `GET /api/v4/channels/{channel_id}/posts?since={unix_ms}&page={page}&per_page={per_page}` (To fetch recent posts)
- `POST /api/v4/users/ids` (To resolve author user IDs)

**To satisfy the strict constraint that the tool must not mark any messages as read or viewed**, view-marking endpoints under channel views and unread-state retrievals are intentionally **never called**.

---

## Release History

### 0.6.0 (2026-04-29)
- **Dynamic Gemini Fallback:** Implemented real-time model discovery via the Gemini API to handle rate limits and model deprecations.
- **Improved Error Classification:** Added intelligent retry logic and model-switching based on HTTP error categories.
- **Live Fallback UX:** Added Telegram bot progress updates when switching to fallback models.
- **Unit Testing:** Added robust testing for Gemini model ranking and error categorization.

### 0.5.0 (2026-04-27)
- **Telegram Bot Mode:** Introduced long-polling bot for interactive digests and system health monitoring.
- **System Status Command:** Non-privileged `/status` command for real-time machine health insights.
- **Enhanced Progress Indicators:** Smooth progress bars in both CLI and Telegram interfaces.

### 0.4.0 (2026-04-23)
- **History Management:** Added `history.txt` to track context between digest runs and avoid redundant summaries.
- **Improved HTML Templates:** Significant aesthetic upgrades to the emailed reports.

