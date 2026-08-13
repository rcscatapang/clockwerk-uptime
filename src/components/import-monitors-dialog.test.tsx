import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ImportMonitorsDialog } from "@/components/import-monitors-dialog";
import type { SyncPlan } from "@/lib/tauri";

vi.mock("@/lib/tauri", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/tauri")>("@/lib/tauri");
  return {
    ...actual,
    previewMonitorSync: vi.fn(),
    applyMonitorSync: vi.fn(),
  };
});

import { applyMonitorSync, previewMonitorSync } from "@/lib/tauri";

const previewMock = vi.mocked(previewMonitorSync);
const applyMock = vi.mocked(applyMonitorSync);

const plan = (overrides: Partial<SyncPlan> = {}): SyncPlan => ({
  deleteMissing: false,
  toAdd: ["https://new.test"],
  toUpdate: ["https://changed.test"],
  toDelete: [],
  unchanged: ["https://same.test"],
  ...overrides,
});

function renderDialog(deleteMissing = false) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const onDeleteMissingChange = vi.fn();
  render(
    <QueryClientProvider client={queryClient}>
      <ImportMonitorsDialog
        path="/tmp/monitors.json"
        onOpenChange={() => {}}
        deleteMissing={deleteMissing}
        onDeleteMissingChange={onDeleteMissingChange}
      />
    </QueryClientProvider>,
  );
  return { onDeleteMissingChange };
}

beforeEach(() => {
  previewMock.mockReset();
  applyMock.mockReset();
});

describe("ImportMonitorsDialog", () => {
  it("renders the plan groups and applies only on confirm", async () => {
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    previewMock.mockResolvedValue(plan());
    applyMock.mockResolvedValue({
      added: 1,
      updated: 1,
      deleted: 0,
      unchanged: 1,
    });
    renderDialog();

    expect(await screen.findByText("Add (1)")).toBeInTheDocument();
    expect(screen.getByText("Update (1)")).toBeInTheDocument();
    expect(screen.getByText("Unchanged (1)")).toBeInTheDocument();
    expect(screen.getByText("https://new.test")).toBeInTheDocument();
    // Delete group is hidden while delete-missing is off.
    expect(screen.queryByText(/^Delete \(/)).not.toBeInTheDocument();
    expect(applyMock).not.toHaveBeenCalled();

    await user.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(applyMock).toHaveBeenCalledOnce());
    expect(applyMock).toHaveBeenCalledWith("/tmp/monitors.json", false);
  });

  it("lists deletions with their history consequence when delete-missing is on", async () => {
    previewMock.mockResolvedValue(
      plan({ deleteMissing: true, toDelete: ["https://gone.test"] }),
    );
    renderDialog(true);

    expect(await screen.findByText("Delete (1)")).toBeInTheDocument();
    expect(
      screen.getByText(/recorded check history is removed/i),
    ).toBeInTheDocument();
    expect(previewMock).toHaveBeenCalledWith("/tmp/monitors.json", true);
  });

  it("blocks apply when the file matches the current monitors", async () => {
    previewMock.mockResolvedValue(plan({ toAdd: [], toUpdate: [] }));
    renderDialog();

    expect(await screen.findByText(/nothing to apply/i)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled();
  });

  it("shows a validation failure in place and blocks apply", async () => {
    previewMock.mockRejectedValue({
      code: "InvalidInput",
      message: "entry 2: unknown field `uptime_status`",
    });
    renderDialog();

    expect(
      await screen.findByText(/entry 2: unknown field/),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Apply" })).toBeDisabled();
  });
});
