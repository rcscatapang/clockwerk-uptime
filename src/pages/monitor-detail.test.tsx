import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter, Route, Routes } from "react-router";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MonitorDetailPage } from "@/pages/monitor-detail";

vi.mock("recharts", () => ({
  CartesianGrid: () => null,
  Line: () => null,
  LineChart: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  ReferenceArea: () => null,
  ResponsiveContainer: ({ children }: { children: React.ReactNode }) => <div>{children}</div>,
  Tooltip: () => null,
  XAxis: () => null,
  YAxis: () => null,
}));

vi.mock("@/lib/tauri", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/tauri")>("@/lib/tauri");
  return {
    ...actual,
    getMonitor: vi.fn(),
    getHistory: vi.fn(),
    getUptimeStats: vi.fn(),
  };
});

import { getHistory, getMonitor, getUptimeStats } from "@/lib/tauri";

const getMonitorMock = vi.mocked(getMonitor);
const getHistoryMock = vi.mocked(getHistory);
const getUptimeStatsMock = vi.mocked(getUptimeStats);

function renderDetail() {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={queryClient}>
      <MemoryRouter initialEntries={["/monitors/1"]}>
        <Routes>
          <Route path="/monitors/:id" element={<MonitorDetailPage />} />
        </Routes>
      </MemoryRouter>
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  getMonitorMock.mockReset();
  getHistoryMock.mockReset();
  getUptimeStatsMock.mockReset();
  getMonitorMock.mockResolvedValue({
    id: 1,
    url: "https://example.com",
    uptimeCheckEnabled: true,
    checkIntervalMinutes: 5,
    checkMethod: "GET",
    lookForString: "",
    uptimeStatus: "down",
    uptimeFailureReason: "timeout",
    consecutiveFailures: 2,
    statusLastChangeAt: "2026-08-10T00:00:00.000Z",
    lastCheckAt: "2026-08-11T00:00:00.000Z",
    downAlertSentAt: "2026-08-10T00:00:00.000Z",
    certCheckEnabled: true,
    certStatus: "not_yet_checked",
    certExpiresAt: null,
    certIssuer: null,
    certFailureReason: null,
    createdAt: "2026-08-01T00:00:00.000Z",
    updatedAt: "2026-08-11T00:00:00.000Z",
    lastResponseTimeMs: 300,
  });
  getUptimeStatsMock.mockResolvedValue({
    uptime24h: 75,
    uptime7d: 90,
    uptime30d: 99,
    avgResponseTimeMs24h: 200,
    lastCheckAt: "2026-08-11T00:00:00.000Z",
    currentStatus: "down",
  });
  getHistoryMock.mockResolvedValue({
    points: [],
    incidents: [
      {
        startedAt: "2026-08-10T00:00:00.000Z",
        endedAt: null,
        durationSeconds: 3600,
        failureReason: "timeout",
        ongoing: true,
        includesGap: true,
      },
    ],
  });
});

describe("MonitorDetailPage", () => {
  it("renders incident and certificate states and refetches by range", async () => {
    const user = userEvent.setup();
    renderDetail();

    expect(await screen.findByText("https://example.com")).toBeInTheDocument();
    expect(screen.getByText("Ongoing")).toBeInTheDocument();
    expect(screen.getByText(/includes monitoring gap/i)).toBeInTheDocument();
    expect(screen.getByText("Not checked yet")).toBeInTheDocument();
    expect(getHistoryMock).toHaveBeenCalledWith(1, "24h");

    await user.click(screen.getByRole("button", { name: "7 days" }));
    await waitFor(() => expect(getHistoryMock).toHaveBeenCalledWith(1, "7d"));
  });
});
