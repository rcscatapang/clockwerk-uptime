// Tray-first lifecycle
// =====================
// This app lives in the menubar/tray, not in its window:
//
//   - Closing the window HIDES it (see `on_window_event` below) — the process
//     and the background monitoring keep running.
//   - The tray menu is the only way to fully quit: "Quit" calls `app.exit(0)`.
//   - "Open" re-shows and focuses the hidden window.
//   - As a safety net, `RunEvent::ExitRequested` without an exit code (e.g. the
//     last window being destroyed) is vetoed so the process never dies silently;
//     an explicit `app.exit(..)` from the tray passes through.

mod alerter;
mod certificate;
mod checker;
mod engine;
mod error;
mod history;
mod secrets;
mod slack;
mod state;
mod store;

use std::sync::Arc;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Emitter, Manager, State,
};
use tauri_plugin_autostart::ManagerExt;

use checker::CheckContext;
use error::AppError;
use history::{HistoryRange, HistoryResponse, UptimeStats};
use store::{Monitor, MonitorInput, Settings, Store};

#[tauri::command]
fn list_monitors(store: State<Arc<Store>>) -> Result<Vec<Monitor>, AppError> {
    store.list_monitors()
}

#[tauri::command]
fn get_monitor(store: State<Arc<Store>>, id: i64) -> Result<Monitor, AppError> {
    store.get_monitor(id)
}

#[tauri::command]
fn create_monitor(store: State<Arc<Store>>, input: MonitorInput) -> Result<Monitor, AppError> {
    store.create_monitor(&input)
}

#[tauri::command]
fn update_monitor(
    store: State<Arc<Store>>,
    id: i64,
    input: MonitorInput,
) -> Result<Monitor, AppError> {
    store.update_monitor(id, &input)
}

#[tauri::command]
fn delete_monitor(store: State<Arc<Store>>, id: i64) -> Result<(), AppError> {
    store.delete_monitor(id)
}

#[tauri::command]
fn get_uptime_stats(store: State<Arc<Store>>, monitor_id: i64) -> Result<UptimeStats, AppError> {
    store.get_uptime_stats(monitor_id)
}

#[tauri::command]
fn get_history(
    store: State<Arc<Store>>,
    monitor_id: i64,
    range: HistoryRange,
) -> Result<HistoryResponse, AppError> {
    store.get_history(monitor_id, range)
}

/// Run a full check for one monitor immediately, outside the schedule.
#[tauri::command]
async fn check_now(
    app: tauri::AppHandle,
    store: State<'_, Arc<Store>>,
    ctx: State<'_, CheckContext>,
    id: i64,
) -> Result<Monitor, AppError> {
    let recorded = engine::check_one(store.inner(), &ctx.client, &ctx.config, id).await?;
    alerter::handle_check(&app, store.inner(), &recorded).await;
    if let Some(certificate) = engine::check_certificate_one(store.inner(), id).await? {
        alerter::handle_certificate_check(&app, store.inner(), &certificate).await;
    }
    let _ = app.emit(
        engine::CHECK_COMPLETED_EVENT,
        engine::CheckCompletedPayload {
            monitor_ids: vec![id],
        },
    );
    store.get_monitor(id)
}

fn settings_with_secrets(store: &Store) -> Result<Settings, AppError> {
    let mut settings = store.get_settings()?;
    settings.slack_webhook_configured = secrets::get_slack_webhook()?.is_some();
    Ok(settings)
}

/// Verify and store the Slack webhook, or remove it when the value is empty,
/// without ever returning the secret.
#[tauri::command]
async fn set_slack_webhook(
    store: State<'_, Arc<Store>>,
    url: String,
) -> Result<Settings, AppError> {
    let url = url.trim().to_string();
    if url.is_empty() {
        secrets::delete_slack_webhook()?;
        return settings_with_secrets(store.inner());
    }
    slack::validate_webhook_url(&url)?;
    slack::verify(&url).await?;
    secrets::set_slack_webhook(&url)?;
    settings_with_secrets(store.inner())
}

#[tauri::command]
fn get_settings(store: State<Arc<Store>>) -> Result<Settings, AppError> {
    settings_with_secrets(store.inner())
}

/// Persists settings in SQLite and applies side effects (autostart registration).
/// The DB is the source of truth for the toggle; the OS registration follows it.
#[tauri::command]
fn update_settings(
    app: tauri::AppHandle,
    store: State<Arc<Store>>,
    settings: Settings,
) -> Result<Settings, AppError> {
    let autolaunch = app.autolaunch();
    let apply = |enabled: bool| {
        if enabled {
            autolaunch.enable()
        } else {
            autolaunch.disable()
        }
    };
    apply(settings.autostart_enabled)
        .map_err(|e| AppError::Internal(format!("could not update launch at login: {e}")))?;
    if let Err(e) = store.save_settings(&settings) {
        // Keep OS registration and DB in sync: revert the registration
        // (best effort) if persisting failed.
        let _ = apply(!settings.autostart_enabled);
        return Err(e);
    }
    settings_with_secrets(store.inner())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            list_monitors,
            get_monitor,
            create_monitor,
            update_monitor,
            delete_monitor,
            get_uptime_stats,
            get_history,
            check_now,
            set_slack_webhook,
            get_settings,
            update_settings
        ])
        .setup(|app| {
            // Open the store before anything else can touch the DB; migrations
            // run inside `Store::open`.
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let store = Store::open(&data_dir.join("monitor.db"))
                .map_err(|e| format!("failed to open monitor.db: {e}"))?;
            let gap_monitor_ids = store
                .record_launch_gaps()
                .map_err(|e| format!("failed to record launch gaps: {e}"))?;
            if !gap_monitor_ids.is_empty() {
                tracing::info!(count = gap_monitor_ids.len(), "recorded launch monitoring gaps");
            }
            app.manage(Arc::new(store));
            app.manage(CheckContext::default());
            engine::start(app.handle().clone());

            let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;

            TrayIconBuilder::with_id("main-tray")
                .icon(app.default_window_icon().expect("bundled icon").clone())
                // macOS-only hint: render the icon as a monochrome template so it
                // matches the menubar theme. No-op on other platforms.
                .icon_as_template(true)
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            // Close button → hide, keep running (tray-first; see module comment).
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                // Veto implicit exits (no code = not from `app.exit`); the tray
                // menu's Quit passes an explicit code and is allowed through.
                if code.is_none() {
                    api.prevent_exit();
                }
            }
        });
}
