import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { MemoryRouter } from "react-router";
import { describe, expect, it } from "vitest";

import { AppLayout } from "@/components/app-layout";

describe("AppLayout", () => {
  it("opens the Clockwerk about dialog from the sidebar footer", async () => {
    const user = userEvent.setup();

    render(
      <MemoryRouter>
        <AppLayout />
      </MemoryRouter>,
    );

    expect(screen.getByText("v0.1.0")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "About Clockwerk" }));

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByRole("heading", { name: "Clockwerk" })).toBeInTheDocument();
    expect(screen.getByText("Version")).toBeInTheDocument();
    expect(screen.getByText("0.1.0")).toBeInTheDocument();
  });
});
