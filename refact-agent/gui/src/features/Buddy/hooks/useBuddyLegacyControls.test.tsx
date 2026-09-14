import React from "react";
import { Provider } from "react-redux";
import { renderHook, waitFor } from "@testing-library/react";
import { http, HttpResponse } from "msw";
import { describe, expect, it } from "vitest";
import { server } from "../../../utils/mockServer";
import { setUpStore } from "../../../app/store";
import type { OperatorStatus } from "../../../services/refact/buddyOperator";
import {
  isLegacyWorkControl,
  useBuddyLegacyControls,
} from "./useBuddyLegacyControls";
import type { BuddyControl } from "../types";

const config = {
  config: { lspPort: 8001, host: "vscode" as const, themeProps: {} },
};
const project = { project_id: "project", canonical_root: "/scratch/project" };

function status(mode: OperatorStatus["mode"]): OperatorStatus {
  const enabled = mode === "enabled";
  return {
    schema_version: 1,
    mode,
    project: enabled ? project : null,
    worker: enabled
      ? { worker_id: "worker", engine_instance_id: "engine", epoch: 1 }
      : null,
    features: {
      storage: enabled,
      actions: enabled,
      supervision: enabled,
      improvement: enabled,
      editorial: enabled,
    },
  };
}

function renderGate() {
  const store = setUpStore(config);
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <Provider store={store}>{children}</Provider>
  );
  return renderHook(() => useBuddyLegacyControls(), { wrapper });
}

function control(action: string): BuddyControl {
  return { id: `${action}-control`, label: action, action, style: "secondary" };
}

const KNOWN_CONTROL_ACTIONS = [
  "accept_quest",
  "care_clean",
  "care_feed",
  "care_pet",
  "care_play",
  "care_sleep",
  "create_worktrees_pulse",
  "dismiss",
  "dismiss_runtime_event",
  "dismiss_speech",
  "dismiss_suggestion",
  "investigate_error",
  "open_buddy",
  "open_setup",
  "open_setup_agents_md",
  "open_setup_commands",
  "open_setup_mcp",
  "open_setup_mode",
  "open_setup_skills",
  "open_setup_subagents",
  "open_stats",
  "open_worktree_cleanup",
  "open_worktrees",
  "reroll_personality",
  "review_worktree_cleanup",
];
const GATED_CONTROL_ACTIONS = ["create_worktrees_pulse", "dismiss_suggestion"];

describe("useBuddyLegacyControls", () => {
  const modes: [OperatorStatus["mode"], boolean][] = [
    ["disabled", true],
    ["enabled", false],
    ["unavailable", false],
  ];
  it.each(modes)(
    "returns %s gate value for operator mode",
    async (mode, expected) => {
      server.use(
        http.get("*/v1/buddy/operator/status", () =>
          HttpResponse.json(status(mode)),
        ),
      );
      const { result } = renderGate();
      await waitFor(() => expect(result.current).toBe(expected));
      expect(result.current).toBe(expected);
    },
  );

  it("keeps legacy controls hidden when operator authority cannot be read", async () => {
    let requests = 0;
    server.use(
      http.get("*/v1/buddy/operator/status", () => {
        requests += 1;
        return HttpResponse.json({ detail: "unavailable" }, { status: 503 });
      }),
    );
    const { result } = renderGate();
    await waitFor(() => expect(requests).toBe(1));
    await waitFor(() => expect(result.current).toBe(false));
  });

  it("keeps legacy controls hidden while operator authority is still loading", async () => {
    let release: (() => void) | undefined;
    const pending = new Promise<void>((resolve) => {
      release = resolve;
    });
    server.use(
      http.get("*/v1/buddy/operator/status", async () => {
        await pending;
        return HttpResponse.json(status("disabled"));
      }),
    );
    const { result } = renderGate();
    expect(result.current).toBe(false);
    release?.();
    await waitFor(() => expect(result.current).toBe(true));
  });
});

describe("isLegacyWorkControl", () => {
  it("gates exactly the migrated work-resolution control actions", () => {
    const gated = KNOWN_CONTROL_ACTIONS.filter((action) =>
      isLegacyWorkControl(control(action)),
    );
    expect([...gated].sort()).toEqual(GATED_CONTROL_ACTIONS);
    for (const action of GATED_CONTROL_ACTIONS) {
      expect(isLegacyWorkControl(control(action))).toBe(true);
    }
    expect(isLegacyWorkControl(control("resolve_future_work"))).toBe(false);
  });
});
