import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { fireEvent, render, screen, waitFor } from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import { BuddySupervision } from "../BuddySupervision";

const config = {
  config: { host: "vscode" as const, lspPort: 8001, themeProps: {} },
};
const project = { project_id: "p", canonical_root: "/fixture" };
const worker = { worker_id: "w", engine_instance_id: "e", epoch: 1 };
const context = {
  schema_version: 1 as const,
  project,
  worker,
  authenticated_owner: {
    source_kind: "external" as const,
    source_id: "user",
    source_version: "http-v1",
  },
  features: {
    storage: true,
    supervision: true,
    editorial: false,
    actions: false,
    improvement: false,
  },
};

/** The status capability is native-provided: no controls are inferred locally. */
describe("BuddySupervision native controls", () => {
  it("uses a selected pre-authorized note ID and exact native control endpoint", async () => {
    const received: unknown[] = [];
    let attempts = 0;
    const link = {
      schema_version: 1,
      project,
      worker,
      mandate_id: "mandate",
      chat_id: "chat",
      command_acknowledged: true,
      dispatch_attempted: true,
      revoked: false,
      escalation: null,
      delivery_run_id: null,
      pause_requested: false,
      control_history: [],
    };
    server.use(
      http.get("*/v1/buddy/supervision/context", () =>
        HttpResponse.json(context),
      ),
      http.get("*/v1/buddy/supervision/mandates/mandate/status", () =>
        HttpResponse.json({
          project,
          worker,
          link,
          native_goal: { status: "paused" },
          native_worker: { status: "running" },
          native_worker_goal: { status: "paused" },
          coverage: "native only",
          host_wake_supported: false,
          daemon_wake_supported: false,
          allowed_operations: ["steer_worker"],
          steering_notes: [{ note_id: "N1", text: "Keep scope bounded" }],
          native_pause: {
            requested: false,
            quiescent: false,
            running_tool: null,
            running_tool_cancelable: false,
            queued_native_notes: 0,
          },
        }),
      ),
      http.post(
        "*/v1/buddy/supervision/mandates/mandate/message",
        async ({ request }) => {
          received.push(await request.json());
          attempts++;
          if (attempts === 1) return HttpResponse.json({}, { status: 503 });
          return HttpResponse.json(link);
        },
      ),
    );
    render(<BuddySupervision />, { preloadedState: config });
    await screen.findByLabelText("Existing mandate ID");
    fireEvent.change(screen.getByLabelText("Existing mandate ID"), {
      target: { value: "mandate" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: "Observe existing mandate" }),
    );
    const notes = await screen.findByLabelText("Pre-authorized steering note");
    expect(
      screen.getByRole("button", { name: "Send steering note by ID" }),
    ).toBeDisabled();
    fireEvent.change(notes, { target: { value: "N1" } });
    fireEvent.click(
      screen.getByRole("button", { name: "Send steering note by ID" }),
    );
    await waitFor(() => expect(received).toHaveLength(1));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Request unconfirmed",
    );
    expect(
      screen.getByRole("button", { name: "Send steering note by ID" }),
    ).toBeDisabled();
    fireEvent.click(
      screen.getByRole("button", { name: "Refresh native supervision status" }),
    );
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Send steering note by ID" }),
      ).toBeEnabled(),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "Send steering note by ID" }),
    );
    await waitFor(() => expect(received).toHaveLength(2));
    expect(received).toEqual([
      expect.objectContaining({
        control_id: expect.any(String),
        note_id: "N1",
      }),
      expect.objectContaining({
        control_id: (received[0] as { control_id: string }).control_id,
        note_id: "N1",
      }),
    ]);
  });
  it("resets mandate confirmation after form edits and defaults engineering to apply_patch", async () => {
    server.use(
      http.get("*/v1/buddy/supervision/context", () =>
        HttpResponse.json(context),
      ),
    );
    render(<BuddySupervision />, { preloadedState: config });
    const model = await screen.findByLabelText("Exact model ID");
    fireEvent.change(model, { target: { value: "provider/model" } });
    fireEvent.change(screen.getByLabelText(/Mandate expires/), {
      target: { value: "2030-01-01T00:00" },
    });
    const confirmation = screen.getByLabelText(
      /I grant only the exact project/,
    );
    fireEvent.click(confirmation);
    expect(
      screen.getByRole("button", { name: "Grant bounded mandate" }),
    ).toBeEnabled();
    fireEvent.change(model, { target: { value: "provider/other-model" } });
    expect(confirmation).not.toBeChecked();
    expect(
      screen.getByRole("button", { name: "Grant bounded mandate" }),
    ).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Worker scope"), {
      target: { value: "engineering" },
    });
    expect(screen.getByLabelText("apply_patch")).toBeChecked();
  });
});
