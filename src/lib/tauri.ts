// Typed wrapper around Tauri's `invoke`.
//
// The Rust command surface is the app's entire API; every command gets a typed
// function here so components never call `invoke` with raw strings.
//
// Error contract
// --------------
// Every command rejects with an `AppErrorPayload` — `{ code, message }` as
// serialized by Rust's `AppError` (src-tauri/src/error.rs). `code` is stable
// and is what UI code should match on; `message` is a human-readable fallback.
//
//   InvalidUrl   — URL failed to parse or isn't http(s)
//   DuplicateUrl — another monitor already uses this URL
//   InvalidInput — any other validation failure (interval < 1, HEAD + look-for-string, …)
//   NotFound     — no monitor with that id
//   Db           — unexpected database error
//   Internal     — an OS-level operation failed (e.g. autostart registration)

import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

export type AppErrorCode =
  | "InvalidUrl"
  | "DuplicateUrl"
  | "InvalidInput"
  | "NotFound"
  | "Db"
  | "Internal";

export interface AppErrorPayload {
  code: AppErrorCode;
  message: string;
}

export function isAppError(e: unknown): e is AppErrorPayload {
  return (
    typeof e === "object" &&
    e !== null &&
    typeof (e as AppErrorPayload).code === "string" &&
    typeof (e as AppErrorPayload).message === "string"
  );
}

export type CheckMethod = "GET" | "HEAD" | "POST";
export type UptimeStatus = "not_yet_checked" | "up" | "down";
export type CertStatus = "not_yet_checked" | "valid" | "invalid";

export interface Monitor {
  id: number;
  url: string;
  uptimeCheckEnabled: boolean;
  checkIntervalMinutes: number;
  checkMethod: CheckMethod;
  lookForString: string;
  uptimeStatus: UptimeStatus;
  uptimeFailureReason: string | null;
  consecutiveFailures: number;
  statusLastChangeAt: string | null;
  lastCheckAt: string | null;
  downAlertSentAt: string | null;
  certCheckEnabled: boolean;
  certStatus: CertStatus;
  certExpiresAt: string | null;
  certIssuer: string | null;
  certFailureReason: string | null;
  certLastCheckAt: string | null;
  certExpiryAlertSentAt: string | null;
  createdAt: string;
  updatedAt: string;
  /** Response time of the latest real check, if any. */
  lastResponseTimeMs: number | null;
}

export interface MonitorInput {
  url: string;
  checkIntervalMinutes: number;
  checkMethod: CheckMethod;
  lookForString: string;
  uptimeCheckEnabled: boolean;
  /** Omit/null for the scheme default: https → on, http → forced off. */
  certCheckEnabled?: boolean | null;
}

export interface Settings {
  autostartEnabled: boolean;
  slackWebhookConfigured: boolean;
  historyRetentionDays: number;
  lastPruneAt: string | null;
}

export type HistoryRange = "24h" | "7d" | "30d";
export type HistoryStatus = "up" | "down" | "gap" | "mixed";

export interface UptimeStats {
  uptime24h: number | null;
  uptime7d: number | null;
  uptime30d: number | null;
  avgResponseTimeMs24h: number | null;
  lastCheckAt: string | null;
  currentStatus: UptimeStatus;
}

export interface HistoryPoint {
  startedAt: string;
  endedAt: string;
  status: HistoryStatus;
  avgResponseTimeMs: number | null;
}

export interface Incident {
  startedAt: string;
  endedAt: string | null;
  durationSeconds: number;
  failureReason: string | null;
  ongoing: boolean;
  includesGap: boolean;
}

export interface HistoryResponse {
  points: HistoryPoint[];
  incidents: Incident[];
}

export function listMonitors(): Promise<Monitor[]> {
  return invoke<Monitor[]>("list_monitors");
}

export function getMonitor(id: number): Promise<Monitor> {
  return invoke<Monitor>("get_monitor", { id });
}

export function createMonitor(input: MonitorInput): Promise<Monitor> {
  return invoke<Monitor>("create_monitor", { input });
}

export function updateMonitor(id: number, input: MonitorInput): Promise<Monitor> {
  return invoke<Monitor>("update_monitor", { id, input });
}

export function deleteMonitor(id: number): Promise<void> {
  return invoke<void>("delete_monitor", { id });
}

export function checkNow(id: number): Promise<Monitor> {
  return invoke<Monitor>("check_now", { id });
}

export function getUptimeStats(monitorId: number): Promise<UptimeStats> {
  return invoke<UptimeStats>("get_uptime_stats", { monitorId });
}

export function getHistory(
  monitorId: number,
  range: HistoryRange,
): Promise<HistoryResponse> {
  return invoke<HistoryResponse>("get_history", { monitorId, range });
}

/**
 * Sync file format, mirroring `SyncEntry` in src-tauri/src/sync.rs — the legal
 * keys of an import file, snake_cased as they appear in the JSON. Rust is the
 * validating authority; this type documents the contract for the UI.
 */
export interface SyncFileEntry {
  url: string;
  uptime_check_enabled?: boolean;
  check_interval_minutes?: number;
  check_method?: CheckMethod;
  look_for_string?: string;
  cert_check_enabled?: boolean;
}

export interface SyncPlan {
  deleteMissing: boolean;
  toAdd: string[];
  toUpdate: string[];
  toDelete: string[];
  unchanged: string[];
}

export interface SyncResult {
  added: number;
  updated: number;
  deleted: number;
  unchanged: number;
}

/** Native file-open dialog. Returns the chosen path, or null if cancelled. */
export async function pickMonitorSyncFile(): Promise<string | null> {
  const selected = await open({
    multiple: false,
    directory: false,
    title: "Import monitors from JSON",
    filters: [{ name: "JSON", extensions: ["json"] }],
  });
  return typeof selected === "string" ? selected : null;
}

export function previewMonitorSync(
  path: string,
  deleteMissing: boolean,
): Promise<SyncPlan> {
  return invoke<SyncPlan>("preview_monitor_sync", { path, deleteMissing });
}

export function applyMonitorSync(
  path: string,
  deleteMissing: boolean,
): Promise<SyncResult> {
  return invoke<SyncResult>("apply_monitor_sync", { path, deleteMissing });
}

export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export function updateSettings(settings: Settings): Promise<Settings> {
  return invoke<Settings>("update_settings", { settings });
}

export function setSlackWebhook(url: string): Promise<Settings> {
  return invoke<Settings>("set_slack_webhook", { url });
}
