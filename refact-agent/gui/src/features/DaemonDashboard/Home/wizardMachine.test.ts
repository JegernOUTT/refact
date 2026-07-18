import { describe, expect, it } from "vitest";

import type { DaemonWorker } from "../../../services/refact/daemon";
import {
  createWizardState,
  wizardReducer,
  type WizardState,
} from "./wizardMachine";

function worker(projectId: string, state: string): DaemonWorker {
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
  };
}

const empty: WizardState = {
  step: "no_projects",
  projectId: null,
  providerProbeFailed: false,
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

  it("derives initial states from dismissal and worker readiness", () => {
    expect(createWizardState([], false).step).toBe("no_projects");
    expect(createWizardState([worker("a", "starting")], false).step).toBe(
      "project_starting",
    );
    expect(createWizardState([worker("a", "ready")], false)).toMatchObject({
      step: "provider_check",
      projectId: "a",
    });
    expect(createWizardState([], true).step).toBe("done");
  });

  it("returns provider and ready states to waiting when their worker stops", () => {
    const providerState: WizardState = {
      step: "provider_setup_pointer",
      projectId: "a",
      providerProbeFailed: false,
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
    const checking = createWizardState([worker("a", "ready")], false);
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
        { step, projectId: "a", providerProbeFailed: false },
        { type: "skip" },
      ).step,
    ).toBe("done");
  });

  it("restarts a skipped setup against current workers", () => {
    const done = wizardReducer(empty, { type: "skip" });
    expect(
      wizardReducer(done, {
        type: "restart",
        workers: [worker("a", "ready")],
      }),
    ).toMatchObject({ step: "provider_check", projectId: "a" });
  });
});
