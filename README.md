# Mattermost Digest

`mattermost-digest` is a production-quality Rust CLI application that connects to a Mattermost server, fetches messages from your channels over a configurable time window, compiles them into a markdown digest, and sends it to you via email using the Gmail API (OAuth Installed-App flow).

It operates completely transparently to the Mattermost server. It does **not** change any channel's "unread" or "viewed" status.

## Prerequisites
- **Rust toolchain** (1.70+ recommended).
- A **Mattermost Personal Access Token**.
- A **Google Cloud Project** with the Gmail API enabled and an OAuth Installed-App client secret.

## Google OAuth Setup
To send emails via Gmail API, you need Google OAuth credentials:
1. Go to the [Google Cloud Console](https://console.cloud.google.com/).
2. Create a new project or select an existing one.
3. Enable the **Gmail API** for the project.
4. Go to **OAuth consent screen** and configure it (you can set it to "External" and add your own email as a Test User).
5. Go to **Credentials**, click **Create Credentials** -> **OAuth client ID**.
6. Select **Desktop app** (Installed App) as the application type.
7. Download the `client_secret.json` file.
8. Place the `client_secret.json` file in `~/.config/mattermost-digest/client_secret.json` (or any path you prefer, and configure it in `config.toml`).

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

## Build Instructions
Build the project using standard Cargo commands:
```bash
cargo build --release
```
The executable will be located at `target/release/mattermost-digest`.

## Run Instructions

### 1. Test Mattermost
```bash
cargo run -- test mattermost
```
This tests your connection to the Mattermost server using your personal token.

### 2. Authenticate Gmail
```bash
cargo run -- auth gmail
```
This will open your default web browser. Follow the prompts to authenticate with your Google account. It will store the OAuth token cache in `~/.config/mattermost-digest/tokencache.json`. Future runs will use this cached token silently.

### 3. Dry-Run Digest
```bash
cargo run -- run --dry-run
```
This will fetch all new messages, generate the markdown file at `~/.local/state/mattermost-digest/latest-digest.md`, and exit without sending the email.

### 4. Run Digest
```bash
cargo run -- run
```
This fetches messages, creates the markdown file, and sends the digest email to your configured address.

## Limitations and Future Improvements
- Only the past X hours of messages are fetched based on configuration.
- Channel metadata (like Team name) is currently not fully fetched unless needed, prioritizing simple display names.
- Attachments and reactions on posts are currently ignored in the digest.
- In a future version, an HTML alternative body could be sent alongside the text body, utilizing a Markdown to HTML compiler.

## Security Notes
- **Never commit your `config.toml`**, `client_secret.json`, or `tokencache.json`.
- Restrict permissions on your config files (e.g. `chmod 600 ~/.config/mattermost-digest/config.toml`).
- Use tokens with the minimal required permissions on both Google and Mattermost if possible.

## Mattermost REST APIs Used
The application intentionally uses only a read-only subset of the Mattermost API:
- `GET /api/v4/users/me` (To validate the token)
- `GET /api/v4/users/me/channels` (To discover channels)
- `GET /api/v4/channels/{channel_id}/posts?since={unix_ms}&page={page}&per_page={per_page}` (To fetch recent posts)
- `POST /api/v4/users/ids` (To resolve author user IDs)

## Mattermost REST APIs Intentionally Not Used
To satisfy the strict constraint that the tool **must not mark any messages as read or viewed**, these endpoints are intentionally **never called**:
- *Any* view-marking endpoint under channel views (`POST /api/v4/channels/{channel_id}/view`).
- *Any* unread-state retrieval (`GET /api/v4/users/{user_id}/channels/{channel_id}/posts/unread` or `GET /api/v4/users/{user_id}/channels/{channel_id}/unread`).
