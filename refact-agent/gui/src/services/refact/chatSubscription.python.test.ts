import { afterEach, expect, it, vi } from "vitest";
import { subscribeToChatEvents, type EventEnvelope } from "./chatSubscription";
import { pythonApproval, pythonCall } from "../../__tests__/pythonFixtures";

afterEach(() => vi.unstubAllGlobals());

it("filters malformed Python snapshot entries without losing the snapshot or valid siblings", async () => {
  const run = {
    tool_call_id: "t",
    run_id: "r",
    status: "running",
    started_at_ms: 1,
    calls: [pythonCall],
    stdout_tail: "",
  };
  const events: EventEnvelope[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      new Response(
        `data: ${JSON.stringify({
          type: "snapshot",
          chat_id: "chat",
          seq: "1",
          runtime: {
            python_runs: [run, {}],
            python_approvals: [null, pythonApproval],
          },
          messages: [],
        })}\n\n`,
      ),
    ),
  );
  const stop = subscribeToChatEvents(
    "chat",
    { lspPort: 8001 },
    { onEvent: (event) => events.push(event), onError: vi.fn() },
  );
  await vi.waitFor(() => expect(events).toHaveLength(1));
  stop();
  expect(events[0]).toMatchObject({
    runtime: { python_runs: [run], python_approvals: [pythonApproval] },
  });
});

it("maps future call status and approval enums to neutral unknown on live events", async () => {
  const events: EventEnvelope[] = [];
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      new Response(
        `data: ${JSON.stringify({
          type: "python_run_update",
          chat_id: "chat",
          seq: "1",
          tool_call_id: "t",
          run_id: "r",
          update: {
            kind: "call_finished",
            record: { ...pythonCall, status: "future", approval: "future" },
          },
        })}\n\n`,
      ),
    ),
  );
  const stop = subscribeToChatEvents(
    "chat",
    { lspPort: 8001 },
    { onEvent: (event) => events.push(event), onError: vi.fn() },
  );
  await vi.waitFor(() => expect(events).toHaveLength(1));
  stop();
  expect(events[0]).toMatchObject({
    update: { record: { status: "unknown", approval: "unknown" } },
  });
});

it("keeps engine approval requests on the wire and never downgrades an unreadable one", async () => {
  const events: EventEnvelope[] = [];
  const engine = {
    function: "settings.set",
    class: "future-class",
    title: "change engine settings",
    items: [{ label: "a", old: 1, new: 2 }],
  };
  vi.stubGlobal(
    "fetch",
    vi.fn().mockResolvedValue(
      new Response(
        [
          { ...pythonApproval, request_id: "ok", engine },
          { ...pythonApproval, request_id: "bad", engine: { title: 1 } },
        ]
          .map(
            (request, index) =>
              `data: ${JSON.stringify({
                type: "python_approval_requested",
                chat_id: "chat",
                seq: String(index + 1),
                request,
              })}\n\n`,
          )
          .join(""),
      ),
    ),
  );
  const stop = subscribeToChatEvents(
    "chat",
    { lspPort: 8001 },
    { onEvent: (event) => events.push(event), onError: vi.fn() },
  );
  await vi.waitFor(() => expect(events).toHaveLength(2));
  stop();
  expect(events[0]).toMatchObject({
    request: { request_id: "ok", engine: { class: "risky" } },
  });
  expect(events[1]).toMatchObject({
    request: {
      request_id: "bad",
      engine: { class: "risky", title: "change the engine", items: [] },
    },
  });
});
