// Client-side form model for the add/edit monitor form.
//
// Mirrors the backend validation so most mistakes are caught inline before a
// command round-trip; the backend remains the authority and its typed errors
// are mapped back onto fields via `fieldForErrorCode`.

import type { AppErrorCode, CheckMethod, Monitor, MonitorInput } from "@/lib/tauri";

export interface MonitorFormValues {
  url: string;
  checkIntervalMinutes: string; // raw text input; validated to an integer
  checkMethod: CheckMethod;
  lookForString: string;
  uptimeCheckEnabled: boolean;
  certCheckEnabled: boolean;
}

export type MonitorFormErrors = Partial<
  Record<"url" | "checkIntervalMinutes" | "lookForString", string>
>;

export function emptyFormValues(): MonitorFormValues {
  return {
    url: "",
    checkIntervalMinutes: "5",
    checkMethod: "GET",
    lookForString: "",
    uptimeCheckEnabled: true,
    certCheckEnabled: true,
  };
}

export function formValuesFromMonitor(monitor: Monitor): MonitorFormValues {
  return {
    url: monitor.url,
    checkIntervalMinutes: String(monitor.checkIntervalMinutes),
    checkMethod: monitor.checkMethod,
    lookForString: monitor.lookForString,
    uptimeCheckEnabled: monitor.uptimeCheckEnabled,
    certCheckEnabled: monitor.certCheckEnabled,
  };
}

/** The input that round-trips a monitor unchanged through update_monitor. */
export function monitorToInput(monitor: Monitor): MonitorInput {
  return {
    url: monitor.url,
    checkIntervalMinutes: monitor.checkIntervalMinutes,
    checkMethod: monitor.checkMethod,
    lookForString: monitor.lookForString,
    uptimeCheckEnabled: monitor.uptimeCheckEnabled,
    certCheckEnabled: monitor.certCheckEnabled,
  };
}

export function urlScheme(url: string): "http" | "https" | null {
  const trimmed = url.trim().toLowerCase();
  if (trimmed.startsWith("https://")) return "https";
  if (trimmed.startsWith("http://")) return "http";
  return null;
}

export function validateForm(values: MonitorFormValues): MonitorFormErrors {
  const errors: MonitorFormErrors = {};
  const url = values.url.trim();
  if (!url) {
    errors.url = "Enter a URL to monitor.";
  } else if (urlScheme(url) === null) {
    errors.url = "URL must start with http:// or https://.";
  } else {
    try {
      new URL(url);
    } catch {
      errors.url = "This doesn't look like a valid URL.";
    }
  }

  const interval = Number(values.checkIntervalMinutes);
  if (!Number.isInteger(interval) || interval < 1) {
    errors.checkIntervalMinutes = "Interval must be a whole number of at least 1 minute.";
  }

  if (values.checkMethod === "HEAD" && values.lookForString.trim() !== "") {
    errors.lookForString = "A HEAD check has no body to search. Use GET or POST.";
  }

  return errors;
}

export function toMonitorInput(values: MonitorFormValues): MonitorInput {
  const scheme = urlScheme(values.url);
  return {
    url: values.url.trim(),
    checkIntervalMinutes: Number(values.checkIntervalMinutes),
    checkMethod: values.checkMethod,
    lookForString:
      values.checkMethod === "HEAD" ? "" : values.lookForString.trim(),
    uptimeCheckEnabled: values.uptimeCheckEnabled,
    certCheckEnabled: scheme === "https" ? values.certCheckEnabled : false,
  };
}

/**
 * Which form field a backend error code belongs to. Codes without a specific
 * field (InvalidInput, Db, …) surface as a form-level message instead.
 */
export function fieldForErrorCode(
  code: AppErrorCode,
): keyof MonitorFormErrors | null {
  switch (code) {
    case "InvalidUrl":
    case "DuplicateUrl":
      return "url";
    default:
      return null;
  }
}
