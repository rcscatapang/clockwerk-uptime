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
  });
});

describe("SettingsPage alerting", () => {
  it("shows Slack configuration without exposing the stored webhook", async () => {
    getSettingsMock.mockResolvedValue({
      autostartEnabled: false,
      slackWebhookConfigured: true,
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
    });
    getSettingsMock.mockResolvedValue({
      autostartEnabled: false,
      slackWebhookConfigured: true,
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

  it("can clear a configured webhook", async () => {
    const user = userEvent.setup();
    getSettingsMock
      .mockResolvedValueOnce({
        autostartEnabled: false,
        slackWebhookConfigured: true,
      })
      .mockResolvedValue({
        autostartEnabled: false,
        slackWebhookConfigured: false,
      });
    setSlackWebhookMock.mockResolvedValue({
      autostartEnabled: false,
      slackWebhookConfigured: false,
    });
    renderSettings();

    await screen.findByText("Configured");
    await user.click(screen.getByRole("button", { name: "Clear webhook" }));

    await waitFor(() => expect(setSlackWebhookMock).toHaveBeenCalledOnce());
    expect(setSlackWebhookMock.mock.calls[0][0]).toBe("");
    expect(await screen.findByText("Not configured")).toBeInTheDocument();
  });
});
