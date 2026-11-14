/**
 * Required, offline frontend HTTP/SSE integration lane. The owned wire peer tests
 * production command serialization and subscription parsing, not Rust engine or
 * provider correctness. Real-engine compatibility remains in the opt-in sibling
 * chatSubscription.live.integration.test.ts.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  abortGeneration,
  ChatCommandHttpError,
  resetChatParamsSyncState,
  sendChatCommand,
  sendUserMessage,
  updateChatParams,
} from "../../services/refact/chatCommands";
import {
  subscribeToChatEvents,
  type EventEnvelope,
} from "../../services/refact/chatSubscription";
import { startChatSubscriptionFixture } from "./chatSubscriptionFixture";

describe(
  "Chat subscription frontend transport (owned HTTP fixture)",
  { retry: 0 },
  () => {
    let fixture: Awaited<ReturnType<typeof startChatSubscriptionFixture>>;
    const stops: (() => void)[] = [];
    beforeEach(async () => {
      resetChatParamsSyncState();
      fixture = await startChatSubscriptionFixture();
    });
    afterEach(async () => {
      try {
        for (const stop of stops.splice(0)) stop();
        await fixture.close();
        expect(fixture.failures).toEqual([]);
      } finally {
        resetChatParamsSyncState();
      }
    });

    async function connect(chatId: string) {
      const events: EventEnvelope[] = [];
      const errors: Error[] = [];
      const disconnected = vi.fn();
      const activity = vi.fn();
      const stop = subscribeToChatEvents(chatId, fixture.config, {
        onEvent: (event) => events.push(event),
        onError: (error) => errors.push(error),
        onDisconnected: disconnected,
        onActivity: activity,
      });
      stops.push(stop);
      await vi.waitFor(() => expect(events[0]?.type).toBe("snapshot"));
      return { events, errors, disconnected, activity, stop };
    }

    it("sends abort through the command API and returns its request ID", async () => {
      const id = await sendChatCommand("abort", fixture.port, undefined, {
        type: "abort",
      });
      expect(id).toEqual(expect.any(String));
      expect(fixture.requests).toEqual([
        { chatId: "abort", command: { type: "abort", client_request_id: id } },
      ]);
    });

    it("serializes set_params and suppresses already-applied patches", async () => {
      const patch = { model: "fixture-model", mode: "explore" };
      await expect(
        updateChatParams("params", patch, fixture.port),
      ).resolves.toBeUndefined();
      await updateChatParams("params", patch, fixture.port);
      expect(fixture.requests).toHaveLength(1);
      expect(fixture.requests[0].command).toMatchObject({
        type: "set_params",
        patch,
      });
    });

    it("serializes client user-message content and URL-encodes chat IDs", async () => {
      await sendUserMessage("chat /?é", "Hello 🌍", fixture.port);
      expect(fixture.requests[0]).toMatchObject({
        chatId: "chat /?é",
        command: {
          type: "user_message",
          content: "Hello 🌍",
          client_request_id: expect.any(String) as unknown,
        },
      });
    });

    it("reuses explicit client request IDs and accepts fixture applied responses", async () => {
      const chat = await connect("retry");
      const command = {
        type: "set_params" as const,
        patch: { model: "fixture-model" },
      };
      for (let i = 0; i < 2; i++) {
        await expect(
          sendChatCommand(
            "retry",
            fixture.port,
            undefined,
            command,
            false,
            "same-request",
          ),
        ).resolves.toBe("same-request");
      }
      await vi.waitFor(() =>
        expect(
          chat.events.filter((event) => event.type === "ack"),
        ).toHaveLength(2),
      );
      expect(
        fixture.requests.map(({ command: value }) => value.client_request_id),
      ).toEqual(["same-request", "same-request"]);
      expect(chat.events.filter((event) => event.type === "ack")).toEqual(
        expect.arrayContaining([
          expect.objectContaining({
            client_request_id: "same-request",
            accepted: true,
            result: { applied: true },
          }),
        ]),
      );
    });

    it("receives a snapshot through the production subscriber", async () => {
      const chat = await connect("snapshot /é");
      expect(chat.events[0]).toMatchObject({
        chat_id: "snapshot /é",
        seq: "1",
        thread: { id: "snapshot /é" },
        runtime: { state: "idle" },
        messages: [],
      });
      expect(fixture.subscriptions[0].searchParams.get("snapshot_chunks")).toBe(
        "1",
      );
      expect(chat.errors).toEqual([]);
    });

    it("receives late command acknowledgments on an established subscription", async () => {
      const chat = await connect("events");
      await updateChatParams(
        "events",
        { model: "fixture-model" },
        fixture.port,
      );
      await sendUserMessage("events", "Say hi", fixture.port);
      await vi.waitFor(() =>
        expect(chat.events.map((event) => event.type)).toEqual([
          "snapshot",
          "thread_updated",
          "ack",
          "ack",
          "message_added",
          "stream_started",
        ]),
      );
      expect(
        chat.events.find((event) => event.type === "message_added"),
      ).toMatchObject({ message: { content: "Say hi", role: "user" } });
    });

    it("parses fragmented UTF-8/CRLF stream deltas and completion, ignoring invalid frames", async () => {
      const chat = await connect("stream");
      await sendUserMessage("stream", "Say hello", fixture.port);
      await vi.waitFor(() =>
        expect(chat.events.at(-1)?.type).toBe("stream_started"),
      );
      chat.activity.mockClear();
      await fixture.wire(
        "stream",
        ': heartbeat\r\n\r\ndata: not-json\r\n\r\ndata: [DONE]\r\n\r\ndata: {"type":"stream_delta","chat_id":"stream","seq":42,"message_id":"assistant-1","ops":[{"op":"append_content","text":"hé🌍"}]}\r\n\r\n',
      );
      await vi.waitFor(() =>
        expect(chat.events.at(-1)?.type).toBe("stream_delta"),
      );
      expect(chat.activity.mock.calls.length).toBeGreaterThan(1);
      fixture.emit("stream", {
        type: "stream_finished",
        message_id: "assistant-1",
        finish_reason: "stop",
      });
      await vi.waitFor(() =>
        expect(chat.events.at(-1)?.type).toBe("stream_finished"),
      );
      expect(
        chat.events.filter((event) => event.type === "stream_delta"),
      ).toEqual([
        {
          type: "stream_delta",
          chat_id: "stream",
          seq: "42",
          message_id: "assistant-1",
          ops: [{ op: "append_content", text: "hé🌍" }],
        },
      ]);
      expect(chat.errors).toEqual([]);
    });

    it("sends client abort and delivers the fixture removal event", async () => {
      const chat = await connect("abort-stream");
      await sendUserMessage("abort-stream", "Long response", fixture.port);
      await vi.waitFor(() =>
        expect(chat.events.at(-1)?.type).toBe("stream_started"),
      );
      await abortGeneration("abort-stream", fixture.port);
      await vi.waitFor(() =>
        expect(chat.events.at(-1)).toMatchObject({
          type: "message_removed",
          message_id: "assistant-1",
        }),
      );
      expect(
        chat.events.some((event) => event.type === "stream_finished"),
      ).toBe(false);
      expect(fixture.requests.at(-1)?.command.type).toBe("abort");
    });

    it("isolates independent chats and filters misrouted envelopes", async () => {
      const first = await connect("one");
      const second = await connect("two");
      await sendUserMessage("one", "first", fixture.port);
      await sendUserMessage("two", "second", fixture.port);
      await fixture.wire(
        "one",
        'data: {"type":"stream_started","chat_id":"two","seq":"999","message_id":"wrong"}\n\n',
      );
      fixture.emit("one", {
        type: "stream_finished",
        message_id: "assistant-1",
        finish_reason: "stop",
      });
      await vi.waitFor(() => {
        expect(first.events.at(-1)?.type).toBe("stream_finished");
        expect(second.events.at(-1)?.type).toBe("stream_started");
      });
      expect(first.events.every((event) => event.chat_id === "one")).toBe(true);
      expect(second.events.every((event) => event.chat_id === "two")).toBe(
        true,
      );
    });

    it("explicitly re-subscribes after EOF and releases both sessions", async () => {
      const first = await connect("reconnect");
      fixture.end("reconnect");
      await vi.waitFor(() => expect(first.disconnected).toHaveBeenCalledOnce());
      const second = await connect("reconnect");
      expect(second.events[0].seq).toBe("2");
      fixture.emit("reconnect", { type: "stream_started", message_id: "new" });
      await vi.waitFor(() =>
        expect(second.events.at(-1)?.type).toBe("stream_started"),
      );
      expect(first.events).toHaveLength(1);
      second.stop();
      await vi.waitFor(() => expect(fixture.active("reconnect")).toBe(0));
      expect(second.errors).toEqual([]);
    });

    it("cancels subscriptions without delivering late events or abort errors", async () => {
      const chat = await connect("cancel");
      chat.stop();
      fixture.emit("cancel", { type: "stream_started", message_id: "late" });
      await vi.waitFor(() => expect(fixture.active("cancel")).toBe(0));
      expect(chat.events).toHaveLength(1);
      expect(chat.errors).toEqual([]);
      expect(chat.disconnected).not.toHaveBeenCalled();
    });

    it("assembles chunked snapshots before exposing messages to consumers", async () => {
      const chat = await connect("chunks");
      fixture.emit("chunks", {
        ...fixture.snapshot("chunks"),
        seq: "100",
        snapshot_chunked: true,
        chunk_count: 2,
        message_count: 2,
      });
      fixture.emit("chunks", {
        type: "snapshot_chunk",
        seq: "100",
        chunk_index: 0,
        chunk_count: 2,
        messages: [{ role: "user", content: "first" }],
      });
      await fixture.wire("chunks", ": chunk boundary\n\n");
      expect(chat.events).toHaveLength(1);
      fixture.emit("chunks", {
        type: "snapshot_chunk",
        seq: "100",
        chunk_index: 1,
        chunk_count: 2,
        messages: [{ role: "assistant", content: "second" }],
      });
      fixture.emit("chunks", {
        type: "snapshot_end",
        seq: "100",
        chunk_count: 2,
        message_count: 2,
      });
      await vi.waitFor(() => expect(chat.events).toHaveLength(2));
      expect(chat.events[1]).toMatchObject({
        type: "snapshot",
        seq: "100",
        messages: [{ content: "first" }, { content: "second" }],
      });
      expect(chat.errors).toEqual([]);
    });

    it("reports truncated chunked-snapshot EOF without exposing partial messages", async () => {
      const chat = await connect("truncated");
      fixture.emit("truncated", {
        ...fixture.snapshot("truncated"),
        seq: "100",
        snapshot_chunked: true,
        chunk_count: 2,
        message_count: 2,
      });
      fixture.emit("truncated", {
        type: "snapshot_chunk",
        seq: "100",
        chunk_index: 0,
        chunk_count: 2,
        messages: [{ role: "user", content: "partial" }],
      });
      fixture.end("truncated");
      await vi.waitFor(() => expect(chat.errors).toHaveLength(1));
      expect(chat.errors[0].message).toBe(
        "SSE stream ended before chunked snapshot completed; reconnecting",
      );
      expect(chat.events).toHaveLength(1);
      expect(chat.disconnected).not.toHaveBeenCalled();
      await vi.waitFor(() => expect(fixture.active("truncated")).toBe(0));
    });

    it("propagates HTTP command rejection instead of silently accepting it", async () => {
      await expect(
        sendUserMessage("reject", "no", fixture.port),
      ).rejects.toBeInstanceOf(ChatCommandHttpError);
      await expect(
        sendUserMessage("reject", "no", fixture.port),
      ).rejects.toMatchObject({
        status: 409,
        message: expect.stringContaining("fixture rejection") as unknown,
      });
    });
  },
);
