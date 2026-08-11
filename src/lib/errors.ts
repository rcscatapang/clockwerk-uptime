import { isAppError } from "@/lib/tauri";
import type { AppErrorCode } from "@/lib/tauri";

const FRIENDLY_MESSAGES: Partial<Record<AppErrorCode, string>> = {
  DuplicateUrl: "This URL is already monitored.",
  NotFound: "That monitor no longer exists.",
  Db: "Something went wrong while saving. Please try again.",
  Internal: "The operation could not be completed.",
};

/**
 * Human-readable message for any command failure. Validation errors
 * (InvalidUrl, InvalidInput) carry a specific message from the backend and
 * pass through as-is; other codes get a fixed friendly message.
 */
export function errorMessage(e: unknown): string {
  if (isAppError(e)) {
    return FRIENDLY_MESSAGES[e.code] ?? e.message;
  }
  return e instanceof Error ? e.message : String(e);
}
