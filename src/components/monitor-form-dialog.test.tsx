import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { MonitorFormDialog } from "@/components/monitor-form-dialog";

vi.mock("@/lib/tauri", async () => {
  const actual =
    await vi.importActual<typeof import("@/lib/tauri")>("@/lib/tauri");
  return {
    ...actual,
    createMonitor: vi.fn(),
    updateMonitor: vi.fn(),
  };
});

import { createMonitor } from "@/lib/tauri";

const createMonitorMock = vi.mocked(createMonitor);

function renderDialog() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <MonitorFormDialog open onOpenChange={() => {}} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  createMonitorMock.mockReset();
});

describe("MonitorFormDialog", () => {
  it("shows inline validation errors instead of submitting", async () => {
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    renderDialog();

    await user.type(screen.getByLabelText("URL"), "ftp://example.com");
    await user.clear(screen.getByLabelText("Interval (minutes)"));
    await user.type(screen.getByLabelText("Interval (minutes)"), "0");
    await user.click(screen.getByRole("button", { name: "Add monitor" }));

    expect(
      screen.getByText(/URL must start with http/i),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/at least 1 minute/i),
    ).toBeInTheDocument();
    expect(createMonitorMock).not.toHaveBeenCalled();
  });

  it("maps a DuplicateUrl server error onto the url field", async () => {
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    createMonitorMock.mockRejectedValue({
      code: "DuplicateUrl",
      message: "a monitor for this URL already exists",
    });
    renderDialog();

    await user.type(screen.getByLabelText("URL"), "https://example.com");
    await user.click(screen.getByRole("button", { name: "Add monitor" }));

    await waitFor(() =>
      expect(
        screen.getByText("This URL is already monitored."),
      ).toBeInTheDocument(),
    );
  });

  it("auto-adjusts the cert toggle from the url scheme", async () => {
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    renderDialog();

    // Re-query after every change: the switch node can be replaced when its
    // disabled state flips.
    const certSwitch = () => screen.getByLabelText("Certificate checks");
    expect(certSwitch()).toBeDisabled();

    await user.type(screen.getByLabelText("URL"), "https://example.com");
    expect(certSwitch()).toBeEnabled();
    expect(certSwitch()).toHaveAttribute("data-state", "checked");

    await user.clear(screen.getByLabelText("URL"));
    await user.type(screen.getByLabelText("URL"), "http://example.com");
    expect(certSwitch()).toBeDisabled();
    expect(certSwitch()).toHaveAttribute("data-state", "unchecked");
  });

  it("disables look-for-string for HEAD without submitting stale text", async () => {
    const user = userEvent.setup({ pointerEventsCheck: 0 });
    createMonitorMock.mockResolvedValue({} as never);
    renderDialog();

    await user.type(screen.getByLabelText("URL"), "https://example.com");
    await user.type(screen.getByLabelText("Look for string"), "hello");

    // The method select is the only combobox in the dialog.
    const trigger = screen.getByRole("combobox");
    await user.click(trigger);
    await user.click(await screen.findByRole("option", { name: "HEAD" }));

    const lookFor = screen.getByLabelText("Look for string");
    expect(lookFor).toBeDisabled();
    expect(lookFor).toHaveValue("");

    await user.click(screen.getByRole("button", { name: "Add monitor" }));
    await waitFor(() => expect(createMonitorMock).toHaveBeenCalledOnce());
    expect(createMonitorMock.mock.calls[0][0]).toMatchObject({
      checkMethod: "HEAD",
      lookForString: "",
    });
  });
});
