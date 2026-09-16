import { afterEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { HooksPanel } from "./HooksPanel";
import type {
  HookActivity,
  HookRegistration,
  HookSurface,
} from "../../../services/refact/hookSurfaceTypes";

afterEach(() => vi.restoreAllMocks());

const renderPanel = () =>
  render(<HooksPanel />, {
    preloadedState: {
      config: {
        host: "ide",
        lspPort: 8001,
        themeProps: { appearance: "dark" },
      },
    },
  });

function surfaceHandler(surface: Record<string, unknown> | HookSurface) {
  server.use(http.get("*/v1/hooks", () => HttpResponse.json(surface)));
}

const registration = (
  overrides: Partial<HookRegistration> = {},
): HookRegistration => ({
  event: "PreToolUse",
  kind: "control",
  command: "check.sh",
  source: "global refact",
  trust: "global",
  runs: true,
  ...overrides,
});

const surface = (overrides: Partial<HookSurface> = {}): HookSurface => {
  const registrations = overrides.registrations ?? [];
  return {
    enabled: true,
    recent: [],
    runnable: registrations.filter((entry) => entry.runs).length,
    blocked: registrations.filter((entry) => !entry.runs).length,
    denials: 0,
    max_recursion_depth: 2,
    generated_at_ms: 1_000,
    ...overrides,
    registrations,
    registered: registrations.length,
  };
};

const activity = (overrides: Partial<HookActivity> = {}): HookActivity => ({
  event: "PreToolUse",
  kind: "control",
  verdict: "allow",
  runs: [],
  at_ms: 1_000,
  ...overrides,
});

describe("Hooks panel", () => {
  it("renders an explicit unavailable state rather than an empty success", async () => {
    server.use(
      http.get("*/v1/hooks", () =>
        HttpResponse.json({ code: "unauthorized" }, { status: 401 }),
      ),
    );
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Hooks unavailable",
    );
    expect(screen.queryByLabelText("Registered hooks")).toBeNull();
  });

  it("says nothing is registered when nothing is registered", async () => {
    surfaceHandler(surface());
    renderPanel();
    expect(await screen.findByText("No hooks registered")).toBeInTheDocument();
    expect(
      screen.getByText("No hook has fired since this engine started."),
    ).toBeInTheDocument();
  });

  it("lists one registered hook with its event and source", async () => {
    surfaceHandler(surface({ registrations: [registration()] }));
    renderPanel();
    expect(await screen.findByText("check.sh")).toBeInTheDocument();
    expect(
      screen.getByText("PreToolUse · Control · Global"),
    ).toBeInTheDocument();
    expect(screen.getByText("Runs")).toBeInTheDocument();
  });

  it("renders an untrusted project hook as untrusted and not executed", async () => {
    surfaceHandler(
      surface({
        registrations: [
          registration(),
          registration({
            command: "project.sh",
            source: "project refact /repo",
            trust: "untrusted_project",
            runs: false,
            blocked_reason:
              "this project is not listed in hooks.trusted_projects, so this hook is never executed",
          }),
        ],
      }),
    );
    renderPanel();
    expect(await screen.findByText("project.sh")).toBeInTheDocument();
    expect(screen.getByText("Never executed")).toBeInTheDocument();
    expect(
      screen.getByText(/not listed in hooks.trusted_projects/),
    ).toBeInTheDocument();
    expect(
      screen.getByText("PreToolUse · Control · Untrusted project"),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/2 registered · 1 run · 1 never executed/),
    ).toBeInTheDocument();
  });

  it("refuses a snapshot that credits a run to a hook it says never executes", async () => {
    surfaceHandler(
      surface({
        registrations: [
          registration({
            command: "project.sh",
            trust: "untrusted_project",
            runs: false,
            blocked_reason: "never executed",
          }),
        ],
        recent: [
          activity({
            runs: [
              {
                command: "project.sh",
                source: "project refact /repo",
                trust: "untrusted_project",
                outcome: "allow",
                decided: false,
              },
            ],
          }),
        ],
      }),
    );
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Hooks unavailable",
    );
  });

  it("identifies the hook that denied and reflects deny precedence", async () => {
    surfaceHandler(
      surface({
        registrations: [registration(), registration({ command: "block.sh" })],
        denials: 1,
        recent: [
          activity({
            verdict: "deny",
            decided_by: "block.sh",
            reason: "no rm",
            tool: "shell",
            runs: [
              {
                command: "check.sh",
                source: "global refact",
                trust: "global",
                outcome: "allow",
                decided: false,
              },
              {
                command: "block.sh",
                source: "global refact",
                trust: "global",
                outcome: "deny",
                detail: "no rm",
                decided: true,
              },
            ],
          }),
        ],
      }),
    );
    renderPanel();
    expect(await screen.findByLabelText("Deciding hook")).toHaveTextContent(
      "block.sh decided this: Deny. Deny outranks every other answer.",
    );
    expect(
      screen.getByLabelText("Hooks that ran for PreToolUse"),
    ).toHaveTextContent("check.sh returned Allow");
    expect(screen.getByLabelText("Recent denials")).toHaveTextContent(
      "1 recent event were denied or refused",
    );
  });

  it("shows a recursion-cap hit and a refused rewrite instead of staying silent", async () => {
    surfaceHandler(
      surface({
        denials: 2,
        recent: [
          activity({
            verdict: "recursion_blocked",
            reason:
              "hook recursion limit reached for PreToolUse: depth 2 exceeds max_recursion_depth 2",
          }),
          activity({
            verdict: "rewrite_refused",
            kind: "rewrite_revalidation",
            tool: "shell",
            reason:
              "hook rewrite refused: 'shell' is outside the accepted tool authority",
          }),
        ],
      }),
    );
    renderPanel();
    expect(
      await screen.findByText("Recursion cap reached"),
    ).toBeInTheDocument();
    expect(screen.getByText("Rewrite refused")).toBeInTheDocument();
    expect(
      screen.getByText(/outside the accepted tool authority/),
    ).toBeInTheDocument();
  });

  it("says no hook objected when nothing decided an allow", async () => {
    surfaceHandler(
      surface({
        registrations: [registration()],
        recent: [
          activity({
            runs: [
              {
                command: "check.sh",
                source: "global refact",
                trust: "global",
                outcome: "allow",
                decided: false,
              },
            ],
          }),
        ],
      }),
    );
    renderPanel();
    expect(await screen.findByText("No hook objected.")).toBeInTheDocument();
    expect(screen.queryByLabelText("Deciding hook")).toBeNull();
  });
});
