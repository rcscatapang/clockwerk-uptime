<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="src/assets/clockwerk-lockup-dark.png">
    <img src="src/assets/clockwerk-lockup-light.png" alt="Clockwerk" width="281">
  </picture>
</h1>

<p align="center"><strong>Uptime &amp; SSL certificate monitoring for macOS.</strong></p>

A macOS menubar desktop app for monitoring HTTP(S) uptime and SSL certificates. Built with Tauri 2: a Rust core does all the monitoring; a React UI presents it.

> Internal tool. Single-user, local-only data, no server component.

## What it does

- **HTTP(S) uptime checks**: per-monitor interval (default 5 min), GET/HEAD/POST, optional "look for string" body assertion, response-time measurement on every check.
- **SSL certificate checks**: daily validity/expiry/issuer inspection, warning when a certificate expires within 10 days.
- **Alerting**: native macOS notifications and Slack (incoming webhook) when a monitor goes down (after 2 consecutive failures), hourly while it stays down, and on recovery.
- **History & stats**: every check result is stored locally (90-day retention). Uptime percentages (24h / 7d / 30d), latency charts, incident timeline.
- **Honest gaps**: on launch, silence longer than twice a monitor's interval is recorded as a distinct "no data" span. Gaps appear gray and are excluded from uptime and downtime totals; history older than 90 days is pruned daily in small batches.
- **Tray-first**: closing the window hides it; monitoring continues in the background. Optional launch-at-login. Quit from the tray menu.

## Architecture

```
┌─ Tauri app ────────────────────────────────────────────┐
│  Rust core (owns everything stateful)                  │
│  ├─ Scheduler: tokio background task, ticks every 30s  │
│  ├─ Checker: reqwest (HTTP) + SSL cert inspection      │
│  ├─ Store: rusqlite → app-data-dir/monitor.db (SQLite) │
│  ├─ Alerter: native notifications + Slack webhook POST │
│  └─ Secrets: keyring crate → macOS Keychain            │
│           ▲ Tauri commands + events ▼                  │
│  React UI: pure viewer/editor, no direct DB/network    │
└────────────────────────────────────────────────────────┘
```

Design rules:

- **Single DB owner**: only Rust touches SQLite. The frontend reads and writes exclusively through Tauri commands.
- **All network I/O in Rust**: the webview makes no HTTP requests of its own.
- **Secrets in Keychain**: the Slack webhook URL never lands in SQLite and is never sent to the frontend.
- **Minimal Tauri capabilities**: no shell, no filesystem access from the frontend.

## Alerting behavior

| Event | Native notification | Slack |
|---|---|---|
| Monitor goes down after two consecutive failures | Sent | Sent when configured |
| Monitor remains down for 60 minutes | Sent again every hour | Sent again every hour when configured |
| Monitor recovers after a delivered down alert | Sent with downtime | Sent with downtime when configured |
| Certificate becomes invalid | Sent once per invalid transition | Sent once per invalid transition when configured |
| Valid certificate expires within 10 days | Sent at most once per day | Sent at most once per day when configured |

A recovery stays silent when the preceding failures never produced a down
alert. Saving a Slack webhook sends a test message before the URL is stored in
macOS Keychain; removing it leaves native alerts enabled.

Certificate checks run daily for enabled HTTPS monitors and immediately on the
first scheduler tick after creation or enabling. They use the platform trust
store and retain the leaf certificate's issuer and expiry when available.

## Stack

| Layer | Choice |
|---|---|
| Shell | Tauri 2 |
| Core | Rust: tokio, reqwest, rusqlite, keyring |
| UI | React 19, TypeScript, Vite |
| Styling | Tailwind CSS v4, shadcn/ui |
| Data fetching | TanStack Query (over Tauri commands, event-invalidated) |
| Charts | Recharts |

## Development

Prerequisites: Rust (stable, via rustup), Node.js ≥ 22, Xcode Command Line Tools.

```sh
npm install
npm run tauri dev     # run the app with hot reload
npm run tauri build   # produce a local .app / .dmg
```

GitHub Actions runs the frontend and Rust tests and builds on pull requests to
`main`. Run the same checks locally before committing:

```sh
npm test && npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
```

The SQLite database lives in the app data directory (`~/Library/Application Support/<bundle-id>/monitor.db`).
