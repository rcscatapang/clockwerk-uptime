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

use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};
use tauri_plugin_autostart::ManagerExt;

/// App settings exposed to the frontend. Held in memory / OS state for now;
/// issue 02 moves persistence into SQLite.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy)]
pub struct Settings {
    pub autostart_enabled: bool,
}

#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> Result<Settings, String> {
    let autostart_enabled = app.autolaunch().is_enabled().map_err(|e| e.to_string())?;
    Ok(Settings { autostart_enabled })
}

#[tauri::command]
fn update_settings(app: tauri::AppHandle, settings: Settings) -> Result<Settings, String> {
    let autolaunch = app.autolaunch();
    let currently_enabled = autolaunch.is_enabled().map_err(|e| e.to_string())?;
    if settings.autostart_enabled != currently_enabled {
        if settings.autostart_enabled {
            autolaunch.enable().map_err(|e| e.to_string())?;
        } else {
            autolaunch.disable().map_err(|e| e.to_string())?;
        }
    }
    get_settings(app)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .invoke_handler(tauri::generate_handler![get_settings, update_settings])
        .setup(|app| {
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
