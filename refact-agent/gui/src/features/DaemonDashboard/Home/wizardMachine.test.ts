import { describe, expect, it } from "vitest";

import type { DaemonWorker } from "../../../services/refact/daemon";
import {
  createWizardState,
  wizardReducer,
  type WizardState,
} from "./wizardMachine";

function worker(
  projectId: string,
  state: string,
  extra: Partial<DaemonWorker> = {},
): DaemonWorker {
  return {
    project_id: projectId,
    slug: projectId,
    root: `/work/${projectId}`,
    pinned: false,
    last_active_ms: 1,
    state,
    pid: state === "ready" ? 1 : null,
    http_port: state === "ready" ? 8001 : null,
    lsp_port: state === "ready" ? 9001 : null,
    lsp_clients: 0,
    busy_chats: 0,
    exec_running: 0,
    live_proxy_streams: 0,
    cron_next_fire_ms: null,
    idle_deadline_ms: null,
    last_status_report_ms: null,
    last_error: null,
    ...extra,
  };
}

const empty: WizardState = {
  step: "no_projects",
  projectId: null,
  providerProbeFailed: false,
  established: false,
};

describe("wizardMachine", () => {
  it("covers the three-screen path from no project to first chat", () => {
    const adding = wizardReducer(empty, { type: "add_project" });
    expect(adding.step).toBe("adding_project");

    const starting = wizardReducer(adding, { type: "project_opening" });
    expect(starting.step).toBe("project_starting");

    const opened = wizardReducer(starting, {
      type: "project_opened",
      projectId: "refact",
    });
    const checking = wizardReducer(opened, {
      type: "workers_updated",
      workers: [worker("refact", "ready")],
    });
    expect(checking).toMatchObject({
      step: "provider_check",
      projectId: "refact",
    });

    const providerPointer = wizardReducer(checking, {
      type: "providers_checked",
      configured: false,
    });
    expect(providerPointer.step).toBe("provider_setup_pointer");

    const rechecking = wizardReducer(providerPointer, {
      type: "recheck_providers",
    });
    expect(rechecking.step).toBe("provider_check");
    expect(
      wizardReducer(rechecking, {
        type: "providers_checked",
        configured: true,
      }).step,
    ).toBe("ready_for_chat");
  });

  it("keeps fresh installs in the first step and honors dismissal", () => {
    expect(createWizardState([], false).step).toBe("no_projects");
    expect(createWizardState([], true).step).toBe("done");
  });

  it.each(["stopped", "starting", "crashed"])(
    "auto-completes established installs with only %s workers",
    (state) => {
      expect(createWizardState([worker("a", state)], false)).toMatchObject({
        step: "done",
        established: true,
      });
    },
  );

  it("marks installs with pre-existing projects as established", () => {
    expect(createWizardState([], false).established).toBe(false);
    expect(createWizardState([worker("a", "ready")], false).established).toBe(
      true,
    );
    expect(
      createWizardState([worker("a", "ready")], false, true).established,
    ).toBe(false);
  });

  it("keeps the fresh-install flow unchanged by the short-circuit", () => {
    expect(wizardReducer(empty, { type: "chats_detected" })).toBe(empty);
    const opened = wizardReducer(empty, {
      type: "project_opened",
      projectId: "refact",
    });
    const checking = wizardReducer(opened, {
      type: "workers_updated",
      workers: [worker("refact", "ready")],
    });
    expect(checking.established).toBe(false);
    expect(
      wizardReducer(checking, { type: "providers_checked", configured: true })
        .step,
    ).toBe("ready_for_chat");
  });

  it("never short-circuits a user-requested setup run", () => {
    const requested = createWizardState([worker("a", "stopped")], false, true);
    expect(requested).toMatchObject({
      step: "project_starting",
      established: false,
    });
  });

  it("prefers pinned, then most recently active, then first ready project", () => {
    expect(
      createWizardState(
        [
          worker("recent", "ready", { last_active_ms: 100 }),
          worker("pinned", "ready", { pinned: true, last_active_ms: 1 }),
        ],
        false,
        true,
      ).projectId,
    ).toBe("pinned");
    expect(
      createWizardState(
        [
          worker("old", "ready", { last_active_ms: 1 }),
          worker("new", "ready", { last_active_ms: 50 }),
        ],
        false,
        true,
      ).projectId,
    ).toBe("new");
    expect(
      createWizardState(
        [
          worker("starting", "starting"),
          worker("first", "ready", { last_active_ms: null }),
          worker("second", "ready", { last_active_ms: null }),
        ],
        false,
        true,
      ).projectId,
    ).toBe("first");
  });

  it("returns provider and ready states to waiting when their worker stops", () => {
    const providerState: WizardState = {
      step: "provider_setup_pointer",
      projectId: "a",
      providerProbeFailed: false,
      established: false,
    };
    expect(
      wizardReducer(providerState, {
        type: "workers_updated",
        workers: [worker("a", "starting")],
      }).step,
    ).toBe("project_starting");

    expect(
      wizardReducer(
        { ...providerState, step: "ready_for_chat" },
        {
          type: "workers_updated",
          workers: [worker("a", "crashed")],
        },
      ).step,
    ).toBe("project_starting");
  });

  it("falls back to the provider pointer when the probe fails", () => {
    const checking = createWizardState([worker("a", "ready")], false, true);
    expect(
      wizardReducer(checking, { type: "providers_check_failed" }),
    ).toMatchObject({
      step: "provider_setup_pointer",
      providerProbeFailed: true,
    });
  });

  it.each([
    "no_projects",
    "adding_project",
    "project_starting",
    "provider_check",
    "provider_setup_pointer",
    "ready_for_chat",
  ] as const)("allows setup to be skipped from %s", (step) => {
    expect(
      wizardReducer(
        {
          step,
          projectId: "a",
          providerProbeFailed: false,
          established: false,
        },
        { type: "skip" },
      ).step,
    ).toBe("done");
  });

  it("restarts a skipped setup against current workers without short-circuiting", () => {
    const done = wizardReducer(empty, { type: "skip" });
    const restarted = wizardReducer(done, {
      type: "restart",
      workers: [worker("a", "ready")],
    });
    expect(restarted).toMatchObject({
      step: "provider_check",
      projectId: "a",
      established: false,
    });
  });
});
