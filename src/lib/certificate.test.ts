import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { EXPIRY_WARNING_DAYS, certificateState } from "@/lib/certificate";
import type { Monitor } from "@/lib/tauri";

const NOW = new Date("2026-08-15T12:00:00Z");

function monitor(overrides: Partial<Monitor> = {}): Monitor {
  return {
    certCheckEnabled: true,
    certStatus: "valid",
    certExpiresAt: null,
    certIssuer: null,
    certFailureReason: null,
    ...overrides,
  } as Monitor;
}

/** An expiry `days` whole days from NOW. */
function inDays(days: number): string {
  return new Date(NOW.getTime() + days * 86_400_000).toISOString();
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(NOW);
});

afterEach(() => {
  vi.useRealTimers();
});

describe("certificateState", () => {
  it("reports disabled before anything else", () => {
    const state = certificateState(
      monitor({ certCheckEnabled: false, certStatus: "invalid" }),
    );
    expect(state.kind).toBe("disabled");
  });

  it("reports an unchecked certificate", () => {
    expect(certificateState(monitor({ certStatus: "not_yet_checked" })).kind).toBe(
      "not_checked",
    );
  });

  it("carries the failure reason for an invalid certificate", () => {
    const state = certificateState(
      monitor({ certStatus: "invalid", certFailureReason: "self-signed" }),
    );
    expect(state).toMatchObject({ kind: "invalid", reason: "self-signed" });
  });

  it("is valid well before the warning window", () => {
    const state = certificateState(
      monitor({ certExpiresAt: inDays(EXPIRY_WARNING_DAYS + 1) }),
    );
    expect(state.kind).toBe("valid");
  });

  it("warns on the last day of the window, inclusive", () => {
    const state = certificateState(
      monitor({ certExpiresAt: inDays(EXPIRY_WARNING_DAYS) }),
    );
    expect(state).toMatchObject({
      kind: "expiring_soon",
      daysRemaining: EXPIRY_WARNING_DAYS,
    });
  });

  // The backend warns while `0 <= days_remaining <= EXPIRY_WARNING_DAYS`, so
  // the final day still warns rather than reading as expired.
  it("still warns on the day of expiry", () => {
    const state = certificateState(monitor({ certExpiresAt: inDays(0) }));
    expect(state).toMatchObject({ kind: "expiring_soon", daysRemaining: 0 });
  });

  // This is the case the two pages used to disagree on: one showed amber
  // "Expires in 0d", the other red "-1 days".
  it("reports a passed expiry as expired, not as a warning", () => {
    const state = certificateState(monitor({ certExpiresAt: inDays(-3) }));
    expect(state).toMatchObject({ kind: "expired", daysAgo: 3 });
  });

  it("treats a missing expiry on a valid certificate as valid", () => {
    expect(certificateState(monitor({ certExpiresAt: null }))).toMatchObject({
      kind: "valid",
      expiresAt: null,
    });
  });

  it("treats an unparseable expiry as valid rather than throwing", () => {
    expect(certificateState(monitor({ certExpiresAt: "not-a-date" }))).toMatchObject({
      kind: "valid",
      expiresAt: null,
    });
  });
});
