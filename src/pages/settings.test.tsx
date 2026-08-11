import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { SettingsPage } from "@/pages/settings";

vi.mock("@/lib/tauri", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/tauri")>("@/lib/tauri");
  return {
    ...actual,
    getSettings: vi.fn(),
    setSlackWebhook: vi.fn(),
  };
});

import {
  getSettings,
  setSlackWebhook,
} from "@/lib/tauri";

const getSettingsMock = vi.mocked(getSettings);
const setSlackWebhookMock = vi.mocked(setSlackWebhook);

function renderSettings() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <SettingsPage />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  getSettingsMock.mockReset();
  setSlackWebhookMock.mockReset();
  getSettingsMock.mockResolvedValue({
    autostartEnabled: false,
    slackWebhookConfigured: false,
    historyRetentionDays: 90,
    lastPruneAt: null,
  });
});

describe("SettingsPage alerting", () => {
  it("shows the fixed retention window and last prune time", async () => {
    const lastPruneAt = "2026-08-12T01:02:03.000Z";
    getSettingsMock.mockResolvedValue({
      autostartEnabled: false,
      slackWebhookConfigured: false,
      historyRetentionDays: 90,
      lastPruneAt,
    });

    renderSettings();

    expect(await screen.findByText("History retention: 90 days")).toBeInTheDocument();
    expect(
      await screen.findByText(
        `Last pruned: ${new Date(lastPruneAt).toLocaleString()}`,
      ),
    ).toBeInTheDocument();
  });

  it("shows Slack configuration without exposing the stored webhook", async () => {
    getSettingsMock.mockResolvedValue({
      autostartEnabled: false,
      slackWebhookConfigured: true,
      historyRetentionDays: 90,
      lastPruneAt: null,
    });
    renderSettings();

    expect(await screen.findByText("Configured")).toBeInTheDocument();
    expect(screen.getByLabelText("Slack webhook URL")).toHaveValue("");
  });

  it("validates and stores a replacement webhook", async () => {
    const user = userEvent.setup();
    setSlackWebhookMock.mockResolvedValue({
      autostartEnabled: false,
      slackWebhookConfigured: true,
      historyRetentionDays: 90,
      lastPruneAt: null,
    });
    getSettingsMock.mockResolvedValue({
      autostartEnabled: false,
      slackWebhookConfigured: true,
      historyRetentionDays: 90,
      lastPruneAt: null,
    });
    renderSettings();

    const input = screen.getByLabelText("Slack webhook URL");
    await user.type(input, "https://hooks.slack.com/services/T0/B0/secret");
    await user.click(screen.getByRole("button", { name: "Save webhook" }));

    await waitFor(() => expect(setSlackWebhookMock).toHaveBeenCalledOnce());
    expect(setSlackWebhookMock.mock.calls[0][0]).toBe(
      "https://hooks.slack.com/services/T0/B0/secret",
    );
    await waitFor(() => expect(input).toHaveValue(""));
    expect(await screen.findByText("Configured")).toBeInTheDocument();
  });

  it("removes the configured webhook", async () => {
    const user = userEvent.setup();
    getSettingsMock.mockResolvedValue({
      autostartEnabled: false,
      slackWebhookConfigured: true,
      historyRetentionDays: 90,
      lastPruneAt: null,
    });
    setSlackWebhookMock.mockResolvedValue({
      autostartEnabled: false,
      slackWebhookConfigured: false,
      historyRetentionDays: 90,
      lastPruneAt: null,
    });
    renderSettings();

    await user.click(
      await screen.findByRole("button", { name: "Remove webhook" }),
    );

    await waitFor(() => expect(setSlackWebhookMock).toHaveBeenCalledOnce());
    expect(setSlackWebhookMock.mock.calls[0][0]).toBe("");
  });

  it("shows notification permission recovery guidance", async () => {
    renderSettings();

    expect(
      await screen.findByText(/enable notifications for this app/i),
    ).toBeInTheDocument();
  });
});
