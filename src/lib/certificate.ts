// Certificate expiry state, derived once so every view agrees.
//
// The boundary rules mirror the backend deliberately: `certificate_event` in
// `src-tauri/src/store.rs` warns only while `0 <= days_remaining <=
// EXPIRY_WARNING_DAYS`, and treats an already-passed expiry as its own case
// rather than a warning. Views render the state below; none of them recompute
// it, so a threshold change is one edit here plus one in `certificate.rs`.

import type { Monitor } from "@/lib/tauri";

/** Mirrors `EXPIRY_WARNING_DAYS` in `src-tauri/src/certificate.rs`. */
export const EXPIRY_WARNING_DAYS = 10;

const MS_PER_DAY = 86_400_000;

export type CertificateState =
  | { kind: "disabled" }
  | { kind: "not_checked" }
  | { kind: "invalid"; expiresAt: Date | null; reason: string | null }
  | { kind: "expired"; expiresAt: Date; daysAgo: number }
  | { kind: "expiring_soon"; expiresAt: Date; daysRemaining: number }
  | { kind: "valid"; expiresAt: Date | null };

/**
 * Whole days from now until `expiresAt`, rounded up — the same ceiling the
 * backend applies via `(seconds_remaining + 86_399) / 86_400`. Negative once
 * the expiry has passed.
 */
function daysUntil(expiresAt: Date): number {
  return Math.ceil((expiresAt.getTime() - Date.now()) / MS_PER_DAY);
}

function parseExpiry(iso: string | null): Date | null {
  if (!iso) return null;
  const date = new Date(iso);
  return Number.isNaN(date.getTime()) ? null : date;
}

export function certificateState(monitor: Monitor): CertificateState {
  if (!monitor.certCheckEnabled) return { kind: "disabled" };
  if (monitor.certStatus === "not_yet_checked") return { kind: "not_checked" };

  const expiresAt = parseExpiry(monitor.certExpiresAt);

  if (monitor.certStatus === "invalid") {
    return { kind: "invalid", expiresAt, reason: monitor.certFailureReason };
  }

  if (!expiresAt) return { kind: "valid", expiresAt: null };

  const daysRemaining = daysUntil(expiresAt);
  if (daysRemaining < 0) {
    return { kind: "expired", expiresAt, daysAgo: -daysRemaining };
  }
  if (daysRemaining <= EXPIRY_WARNING_DAYS) {
    return { kind: "expiring_soon", expiresAt, daysRemaining };
  }
  return { kind: "valid", expiresAt };
}
