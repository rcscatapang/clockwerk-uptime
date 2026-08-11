// Typed wrapper around Tauri's `invoke`.
//
// The Rust command surface is the app's entire API; every command gets a typed
// function here so components never call `invoke` with raw strings. Later
// issues extend this file as commands are added (see SPEC.md §6).

import { invoke } from "@tauri-apps/api/core";

export interface Settings {
  autostart_enabled: boolean;
}

export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export function updateSettings(settings: Settings): Promise<Settings> {
  return invoke<Settings>("update_settings", { settings });
}
