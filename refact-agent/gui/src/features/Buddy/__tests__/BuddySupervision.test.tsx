import { afterEach, describe, expect, it } from "vitest";
import userEvent from "@testing-library/user-event";
import { resetBuddyPersistentState } from "../ui";
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

HTMLElement.prototype.hasPointerCapture = () => false;
HTMLElement.prototype.releasePointerCapture = () => undefined;
afterEach(() => resetBuddyPersistentState());

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
    fireEvent.click(
      await screen.findByRole("button", { name: /work on its own/ }),
    );
    await screen.findByLabelText("Existing permission ID");
    fireEvent.change(screen.getByLabelText("Existing permission ID"), {
      target: { value: "mandate" },
    });
    fireEvent.click(
      screen.getByRole("button", { name: "Check existing permission" }),
    );
    const notes = await screen.findByRole("combobox", {
      name: "Pre-written note",
    });
    expect(screen.getByRole("button", { name: "Send note" })).toBeDisabled();
    const user = userEvent.setup();
    await user.click(notes);
    await user.click(
      await screen.findByRole("option", { name: "N1: Keep scope bounded" }),
    );
    fireEvent.click(screen.getByRole("button", { name: "Send note" }));
    await waitFor(() => expect(received).toHaveLength(1));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Request unconfirmed",
    );
    expect(screen.getByRole("button", { name: "Send note" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Refresh status" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Send note" })).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Send note" }));
    await waitFor(() => expect(received).toHaveLength(2));
    expect(received).toEqual([
      expect.objectContaining({
        control_id: expect.any(String) as unknown,
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
    fireEvent.click(
      await screen.findByRole("button", { name: /work on its own/ }),
    );
    const model = await screen.findByLabelText("Model ID");
    fireEvent.change(model, { target: { value: "provider/model" } });
    fireEvent.change(screen.getByLabelText(/^Expires/), {
      target: { value: "2030-01-01T00:00" },
    });
    const confirmation = screen.getByLabelText(/I grant only this project/);
    fireEvent.click(confirmation);
    expect(
      screen.getByRole("button", { name: "Grant permission" }),
    ).toBeEnabled();
    fireEvent.change(model, { target: { value: "provider/other-model" } });
    expect(confirmation).not.toBeChecked();
    expect(
      screen.getByRole("button", { name: "Grant permission" }),
    ).toBeDisabled();
    const user = userEvent.setup();
    await user.click(screen.getByRole("combobox", { name: "What it may do" }));
    await user.click(
      await screen.findByRole("option", { name: "Edit listed files" }),
    );
    expect(screen.getByLabelText("apply_patch")).toBeChecked();
  });
});
