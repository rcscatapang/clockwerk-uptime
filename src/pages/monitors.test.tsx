import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MonitorsPage } from "@/pages/monitors";
import type { Monitor } from "@/lib/tauri";

vi.mock("@/lib/tauri", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/tauri")>("@/lib/tauri");
  return {
    ...actual,
    listMonitors: vi.fn(),
    setMonitorsEnabled: vi.fn(),
    deleteMonitors: vi.fn(),
    checkMonitors: vi.fn(),
  };
});

import {
  checkMonitors,
  deleteMonitors,
  listMonitors,
  setMonitorsEnabled,
} from "@/lib/tauri";

const listMonitorsMock = vi.mocked(listMonitors);
const setMonitorsEnabledMock = vi.mocked(setMonitorsEnabled);
const deleteMonitorsMock = vi.mocked(deleteMonitors);
const checkMonitorsMock = vi.mocked(checkMonitors);

function monitor(overrides: Partial<Monitor> & Pick<Monitor, "id" | "url">) {
  return {
    uptimeCheckEnabled: true,
    checkIntervalMinutes: 5,
    checkMethod: "GET",
    lookForString: "",
    uptimeStatus: "up",
    uptimeFailureReason: null,
    consecutiveFailures: 0,
    statusLastChangeAt: null,
    lastCheckAt: null,
    downAlertSentAt: null,
    certCheckEnabled: true,
    certStatus: "valid",
    certExpiresAt: null,
    certIssuer: null,
    certFailureReason: null,
    certLastCheckAt: null,
    certExpiryAlertSentAt: null,
    createdAt: "2026-01-01T00:00:00.000Z",
    updatedAt: "2026-01-01T00:00:00.000Z",
    lastResponseTimeMs: 120,
    ...overrides,
  } as Monitor;
}

const enabled = monitor({ id: 1, url: "https://enabled.test" });
const disabled = monitor({
  id: 2,
  url: "https://disabled.test",
  uptimeCheckEnabled: false,
});

function renderPage() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>
        <MonitorsPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  listMonitorsMock.mockReset();
  setMonitorsEnabledMock.mockReset();
  deleteMonitorsMock.mockReset();
  checkMonitorsMock.mockReset();
  listMonitorsMock.mockResolvedValue([enabled, disabled]);
});

describe("MonitorsPage bulk actions", () => {
  it("hides the bulk bar until something is selected, and clears it after an action", async () => {
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    setMonitorsEnabledMock.mockResolvedValue(2);
    renderPage();

    await screen.findByText("https://enabled.test");
    expect(screen.queryByText(/selected$/)).not.toBeInTheDocument();

    await user.click(screen.getByLabelText("Select all monitors"));
    expect(screen.getByText("2 selected")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Disable" }));
    await waitFor(() => expect(setMonitorsEnabledMock).toHaveBeenCalled());
    expect(setMonitorsEnabledMock.mock.calls[0].slice(0, 2)).toEqual([
      [1, 2],
      false,
    ]);
    await waitFor(() =>
      expect(screen.queryByText("2 selected")).not.toBeInTheDocument(),
    );
  });

  it("checks every enabled monitor for check-all and the raw selection for check-selected", async () => {
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    checkMonitorsMock.mockResolvedValue({ checked: 1, up: 1, down: 0 });
    renderPage();

    await screen.findByText("https://enabled.test");
    await user.click(screen.getByRole("button", { name: /Check all now/ }));
    await waitFor(() => expect(checkMonitorsMock).toHaveBeenCalled());
    expect(checkMonitorsMock.mock.calls[0][0]).toEqual([1]);

    // The disabled monitor is skipped by check-all but honored when selected.
    await user.click(screen.getByLabelText("Select https://disabled.test"));
    await user.click(
      screen.getByRole("button", { name: "Check selected now" }),
    );
    await waitFor(() => expect(checkMonitorsMock).toHaveBeenCalledTimes(2));
    expect(checkMonitorsMock.mock.calls[1][0]).toEqual([2]);
  });

  it("deletes only after the count-stating confirmation", async () => {
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    deleteMonitorsMock.mockResolvedValue(2);
    renderPage();

    await screen.findByText("https://enabled.test");
    await user.click(screen.getByLabelText("Select all monitors"));
    await user.click(screen.getByRole("button", { name: "Delete" }));

    expect(await screen.findByText("Delete 2 monitors?")).toBeInTheDocument();
    expect(deleteMonitorsMock).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Cancel" }));
    expect(deleteMonitorsMock).not.toHaveBeenCalled();
    await waitFor(() =>
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
    );

    await user.click(screen.getByRole("button", { name: "Delete" }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(deleteMonitorsMock).toHaveBeenCalled());
    expect(deleteMonitorsMock.mock.calls[0][0]).toEqual([1, 2]);
  });
});
