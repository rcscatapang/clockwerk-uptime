import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen } from "@testing-library/react";
import { MemoryRouter } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { DashboardPage } from "@/pages/dashboard";
import type { Monitor } from "@/lib/tauri";

vi.mock("@/lib/tauri", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/tauri")>("@/lib/tauri");
  return {
    ...actual,
    listMonitors: vi.fn(),
    getUptimeStats: vi.fn(),
    getHistory: vi.fn(),
  };
});

import { getHistory, getUptimeStats, listMonitors } from "@/lib/tauri";

const listMonitorsMock = vi.mocked(listMonitors);
const getUptimeStatsMock = vi.mocked(getUptimeStats);
const getHistoryMock = vi.mocked(getHistory);

function monitor(id: number, url: string, uptimeStatus: Monitor["uptimeStatus"]): Monitor {
  return {
    id,
    url,
    uptimeCheckEnabled: true,
    checkIntervalMinutes: 5,
    checkMethod: "GET",
    lookForString: "",
    uptimeStatus,
    uptimeFailureReason: uptimeStatus === "down" ? "timeout" : null,
    consecutiveFailures: uptimeStatus === "down" ? 2 : 0,
    statusLastChangeAt: null,
    lastCheckAt: "2026-08-11T00:00:00.000Z",
    downAlertSentAt: null,
    certCheckEnabled: true,
    certStatus: "not_yet_checked",
    certExpiresAt: null,
    certIssuer: null,
    certFailureReason: null,
    createdAt: "2026-08-10T00:00:00.000Z",
    updatedAt: "2026-08-11T00:00:00.000Z",
    lastResponseTimeMs: id === 1 ? 240 : 80,
  };
}

function renderDashboard() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter>
        <DashboardPage />
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  listMonitorsMock.mockReset();
  getUptimeStatsMock.mockReset();
  getHistoryMock.mockReset();
  listMonitorsMock.mockResolvedValue([
    monitor(2, "https://a.example", "up"),
    monitor(1, "https://z.example", "down"),
  ]);
  getUptimeStatsMock.mockImplementation(async (id) => ({
    uptime24h: id === 1 ? 66.7 : null,
    uptime7d: id === 1 ? 92.1 : null,
    uptime30d: id === 1 ? 98.4 : null,
    avgResponseTimeMs24h: id === 1 ? 180 : null,
    lastCheckAt: "2026-08-11T00:00:00.000Z",
    currentStatus: id === 1 ? "down" : "up",
  }));
  getHistoryMock.mockResolvedValue({ points: [], incidents: [] });
});

describe("DashboardPage", () => {
  it("sorts down monitors first and renders uptime states", async () => {
    renderDashboard();

    await screen.findByText("66.7%");
    const cards = await screen.findAllByTestId("monitor-card");
    expect(cards[0]).toHaveTextContent("https://z.example");
    expect(cards[0]).toHaveTextContent("66.7%");
    expect(cards[1]).toHaveTextContent("https://a.example");
    expect(cards[1]).toHaveTextContent("No data");
  });
});
