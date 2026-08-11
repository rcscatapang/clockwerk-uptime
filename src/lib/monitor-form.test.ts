import { describe, expect, it } from "vitest";

import {
  emptyFormValues,
  fieldForErrorCode,
  toMonitorInput,
  urlScheme,
  validateForm,
  type MonitorFormValues,
} from "@/lib/monitor-form";

function values(overrides: Partial<MonitorFormValues>): MonitorFormValues {
  return { ...emptyFormValues(), url: "https://example.com", ...overrides };
}

describe("validateForm", () => {
  it("accepts a plain https monitor", () => {
    expect(validateForm(values({}))).toEqual({});
  });

  it("requires a url", () => {
    expect(validateForm(values({ url: "" })).url).toMatch(/enter a url/i);
  });

  it("rejects non-http(s) schemes", () => {
    for (const url of ["ftp://example.com", "example.com", "file:///etc"]) {
      expect(validateForm(values({ url })).url).toMatch(/http/i);
    }
  });

  it("rejects malformed urls with an http prefix", () => {
    expect(validateForm(values({ url: "https://" })).url).toBeTruthy();
  });

  it("rejects intervals below one minute and non-integers", () => {
    for (const interval of ["0", "-3", "2.5", "abc", ""]) {
      expect(
        validateForm(values({ checkIntervalMinutes: interval }))
          .checkIntervalMinutes,
      ).toBeTruthy();
    }
    expect(
      validateForm(values({ checkIntervalMinutes: "1" })).checkIntervalMinutes,
    ).toBeUndefined();
  });

  it("rejects HEAD combined with look-for-string", () => {
    const errors = validateForm(
      values({ checkMethod: "HEAD", lookForString: "ok" }),
    );
    expect(errors.lookForString).toMatch(/head/i);
    expect(
      validateForm(values({ checkMethod: "GET", lookForString: "ok" }))
        .lookForString,
    ).toBeUndefined();
  });
});

describe("toMonitorInput", () => {
  it("forces cert checks off for http urls", () => {
    const input = toMonitorInput(
      values({ url: "http://example.com", certCheckEnabled: true }),
    );
    expect(input.certCheckEnabled).toBe(false);
  });

  it("keeps the explicit cert choice for https urls", () => {
    expect(
      toMonitorInput(values({ certCheckEnabled: false })).certCheckEnabled,
    ).toBe(false);
    expect(
      toMonitorInput(values({ certCheckEnabled: true })).certCheckEnabled,
    ).toBe(true);
  });

  it("clears look-for-string when the method is HEAD", () => {
    const input = toMonitorInput(
      values({ checkMethod: "HEAD", lookForString: "leftover" }),
    );
    expect(input.lookForString).toBe("");
  });

  it("parses the interval to a number and trims the url", () => {
    const input = toMonitorInput(
      values({ url: "  https://example.com  ", checkIntervalMinutes: "15" }),
    );
    expect(input.url).toBe("https://example.com");
    expect(input.checkIntervalMinutes).toBe(15);
  });
});

describe("urlScheme", () => {
  it("detects schemes case-insensitively", () => {
    expect(urlScheme("HTTPS://x.com")).toBe("https");
    expect(urlScheme("http://x.com")).toBe("http");
    expect(urlScheme("x.com")).toBeNull();
  });
});

describe("fieldForErrorCode", () => {
  it("maps url-shaped codes onto the url field", () => {
    expect(fieldForErrorCode("InvalidUrl")).toBe("url");
    expect(fieldForErrorCode("DuplicateUrl")).toBe("url");
  });

  it("leaves other codes for the form-level message", () => {
    expect(fieldForErrorCode("InvalidInput")).toBeNull();
    expect(fieldForErrorCode("Db")).toBeNull();
    expect(fieldForErrorCode("Internal")).toBeNull();
    expect(fieldForErrorCode("NotFound")).toBeNull();
  });
});
