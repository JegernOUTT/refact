import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Theme } from "@radix-ui/themes";
import { describe, expect, test } from "vitest";

import type { BrowserNetworkEntry } from "../../../services/refact/browser";
import { NetworkPanel } from "./NetworkPanel";

function entry(
  overrides: Partial<BrowserNetworkEntry> = {},
): BrowserNetworkEntry {
  return {
    timestamp: 10,
    method: "GET",
    url: "https://example.com/api/items",
    resource_type: "Fetch",
    status: 200,
    from_service_worker: false,
    is_navigation_request: false,
    ...overrides,
  };
}

function renderPanel(entries: unknown) {
  return render(
    <Theme>
      <NetworkPanel entries={entries} />
    </Theme>,
  );
}

describe("NetworkPanel", () => {
  test("renders request details and distinguishes failed entries", async () => {
    const user = userEvent.setup();
    renderPanel([
      entry({
        timing: { start_time: 10, response_end: 10.125 },
        transfer_size: 1_536,
      }),
      entry({
        url: "https://example.com/api/missing",
        status: 404,
      }),
      entry({
        method: "POST",
        url: "https://example.com/api/save",
        resource_type: "XHR",
        status: 500,
        failure_text: "net::ERR_FAILED",
        redirect_from: "https://example.com/api/legacy-save",
      }),
    ]);

    const trigger = screen.getByRole("button", { name: "Network (3)" });
    expect(trigger).toHaveAttribute("aria-expanded", "false");

    await user.click(trigger);

    expect(screen.getAllByText("GET")).toHaveLength(2);
    expect(screen.getByLabelText("Status 200")).toBeInTheDocument();
    expect(screen.getByText("125 ms")).toBeInTheDocument();
    expect(screen.getByText("1.5 KB")).toBeInTheDocument();
    expect(screen.getAllByText("Fetch")).toHaveLength(2);
    expect(screen.getByText("net::ERR_FAILED")).toBeInTheDocument();
    expect(screen.getByText(/Redirected from/)).toBeInTheDocument();
    expect(screen.getByTestId("network-entry-1")).toHaveAttribute(
      "data-status",
      "error",
    );
    expect(screen.getByTestId("network-entry-1")).toHaveAttribute(
      "data-error",
      "true",
    );
    expect(screen.getByTestId("network-entry-2")).toHaveAttribute(
      "data-status",
      "error",
    );
  });

  test("filters by URL and shows only errors", async () => {
    const user = userEvent.setup();
    renderPanel([
      entry({ url: "https://example.com/api/users" }),
      entry({
        url: "https://example.com/assets/app.css",
        resource_type: "Stylesheet",
        status: 404,
      }),
      entry({
        url: "https://example.com/api/save",
        status: null,
        failure_text: "Blocked by client",
      }),
    ]);

    await user.click(screen.getByRole("button", { name: "Network (3)" }));
    await user.type(screen.getByPlaceholderText("Filter by URL"), "/api/");

    const region = screen.getByRole("region", { name: "Network entries" });
    expect(
      within(region).getByTitle("https://example.com/api/users"),
    ).toBeInTheDocument();
    expect(
      within(region).getByTitle("https://example.com/api/save"),
    ).toBeInTheDocument();
    expect(
      within(region).queryByTitle("https://example.com/assets/app.css"),
    ).toBeNull();

    await user.click(screen.getByRole("switch", { name: "Errors only" }));

    expect(
      within(region).queryByTitle("https://example.com/api/users"),
    ).toBeNull();
    expect(
      within(region).getByTitle("https://example.com/api/save"),
    ).toBeInTheDocument();
    expect(screen.getByText("Blocked by client")).toBeInTheDocument();
  });

  test("renders nothing for an empty entry list", () => {
    const { container } = renderPanel([]);
    expect(container.querySelector("[data-testid='network-panel']")).toBeNull();
  });

  test("tolerates malformed and partial entries", async () => {
    const user = userEvent.setup();
    renderPanel([
      null,
      {},
      { url: 42, status: "bad", timing: { start_time: "early" } },
      { url: "https://example.com/partial", method: "PATCH" },
    ]);

    await user.click(screen.getByRole("button", { name: "Network (3)" }));

    expect(screen.getAllByText("Unknown URL")).toHaveLength(2);
    expect(screen.getByText("PATCH")).toBeInTheDocument();
    expect(
      screen.getByTitle("https://example.com/partial"),
    ).toBeInTheDocument();
    expect(screen.getAllByText("Pending")).toHaveLength(3);
  });
});
