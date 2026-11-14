import { describe, expect, it } from "vitest";
import { screen, within } from "@testing-library/react";
import { render } from "../../utils/test-utils";
import { setUpStore } from "../../app/store";
import { updateConfig } from "../Config/configSlice";
import { RuntimeVersionsSummary } from "./RuntimeVersions";
import type { RuntimeVersions } from "./types";

async function summary(
  versions?: RuntimeVersions | null,
  host: "web" | "vscode" = "vscode",
) {
  const store = setUpStore();
  store.dispatch(updateConfig({ host, runtimeVersions: versions }));
  const result = render(<RuntimeVersionsSummary />, { store });
  await result.user.click(
    screen.getByRole("button", { name: "Component versions" }),
  );
  const slot = (label: string) => {
    const row = screen.getByText(label, { selector: "dt" }).parentElement;
    if (!row) throw new Error(`Missing version row: ${label}`);
    return within(row);
  };
  return { ...result, store, slot };
}

describe("component versions", () => {
  it("keeps legacy hosts and missing metadata unknown without fabricated runtime versions", async () => {
    const { slot } = await summary({
      plugin: { state: "observed", version: "plugin-only" },
      requestedVersion: "policy-only",
    });
    expect(slot("IDE plugin").getByText("plugin-only")).toBeInTheDocument();
    for (const label of [
      "Selected executable",
      "Installed runtime",
      "Staged runtime",
      "Running daemon",
      "Project engine",
    ])
      expect(slot(label).getByText("Not detected")).toBeInTheDocument();
    expect(slot("Plugin UI").getByText("Loaded UI bundle")).toBeInTheDocument();
  });

  it("separates requested policy, selected observation, sources, and stale timestamp", async () => {
    const { slot } = await summary({
      requestedVersion: "2.0",
      selected: { state: "observed", version: "1.0", source: "local" },
      worker: {
        state: "last_observed",
        version: "0.9",
        source: "worker",
        observedAtMs: 1700000000000,
      },
      installed: { state: "observed", commit: "abc" },
    });
    expect(slot("Requested runtime").getByText("2.0")).toBeInTheDocument();
    expect(slot("Selected executable").getByText("1.0")).toBeInTheDocument();
    expect(
      slot("Selected executable").getByText("Local executable"),
    ).toBeInTheDocument();
    expect(
      slot("Project engine").getByText(/Last observed/),
    ).toBeInTheDocument();
    expect(
      slot("Project engine").getByText("Project engine status"),
    ).toBeInTheDocument();
    expect(
      slot("Project engine").getByText(
        new Date(1700000000000).toLocaleString(),
      ),
    ).toHaveAttribute("datetime", new Date(1700000000000).toISOString());
    expect(slot("Installed runtime").getByText("Unknown")).toBeInTheDocument();
  });

  it("labels the web loaded bundle separately", async () => {
    const { slot } = await summary(null, "web");
    expect(slot("Web UI").getByText("Loaded UI bundle")).toBeInTheDocument();
  });

  it("overrides only running daemon with an actual status observation", async () => {
    const store = setUpStore();
    store.dispatch(
      updateConfig({
        runtimeVersions: {
          daemon: { state: "observed", version: "host-daemon" },
          installed: { state: "observed", version: "installed" },
        },
      }),
    );
    const { user } = render(
      <RuntimeVersionsSummary
        daemonOverride={{
          state: "last_observed",
          version: "actual-status",
          source: "daemon",
        }}
      />,
      { store },
    );
    await user.click(
      screen.getByRole("button", { name: "Component versions" }),
    );
    expect(screen.getByText("actual-status")).toBeInTheDocument();
    expect(screen.queryByText("host-daemon")).not.toBeInTheDocument();
    expect(screen.getByText("installed")).toBeInTheDocument();
  });
});
