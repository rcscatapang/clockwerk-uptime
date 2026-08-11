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

mod error;
mod store;

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager, State,
};
use tauri_plugin_autostart::ManagerExt;

use error::AppError;
use store::{Monitor, MonitorInput, Settings, Store};

#[tauri::command]
fn list_monitors(store: State<Store>) -> Result<Vec<Monitor>, AppError> {
    store.list_monitors()
}

#[tauri::command]
fn get_monitor(store: State<Store>, id: i64) -> Result<Monitor, AppError> {
    store.get_monitor(id)
}

#[tauri::command]
fn create_monitor(store: State<Store>, input: MonitorInput) -> Result<Monitor, AppError> {
    store.create_monitor(&input)
}

#[tauri::command]
fn update_monitor(store: State<Store>, id: i64, input: MonitorInput) -> Result<Monitor, AppError> {
    store.update_monitor(id, &input)
}

#[tauri::command]
fn delete_monitor(store: State<Store>, id: i64) -> Result<(), AppError> {
    store.delete_monitor(id)
}

#[tauri::command]
fn get_settings(store: State<Store>) -> Result<Settings, AppError> {
    store.get_settings()
}

/// Persists settings in SQLite and applies side effects (autostart registration).
/// The DB is the source of truth for the toggle; the OS registration follows it.
#[tauri::command]
fn update_settings(
    app: tauri::AppHandle,
    store: State<Store>,
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
    store.get_settings()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            list_monitors,
            get_monitor,
            create_monitor,
            update_monitor,
            delete_monitor,
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
            app.manage(store);

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
