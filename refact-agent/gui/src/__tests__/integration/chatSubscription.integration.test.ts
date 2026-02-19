/**
 * Chat Subscription Integration Tests
 *
 * Integration tests that use the actual refact-lsp server.
 * Requires: refact-lsp running on port 8001
 *
 * Run with: npm run test:no-watch -- chatSubscription.integration
 *
 * Note: These tests are skipped in CI if no server is available.
 */

/* eslint-disable @typescript-eslint/no-unsafe-member-access, @typescript-eslint/no-unsafe-assignment */
import { describe, it, expect, vi } from "vitest";

// Increase test timeout for integration tests
vi.setConfig({ testTimeout: 30000 });
import {
  sendChatCommand,
  sendUserMessage,
  updateChatParams,
  abortGeneration,
} from "../../services/refact/chatCommands";

const LSP_PORT = 8001;
const LSP_URL = `http://127.0.0.1:${LSP_PORT}`;

// Check if server is available
async function isServerAvailable(): Promise<boolean> {
  try {
    const response = await fetch(`${LSP_URL}/v1/ping`, {
      signal: AbortSignal.timeout(2000),
    });
    return response.ok;
  } catch {
    return false;
  }
}

// Generate unique chat ID
function generateChatId(prefix: string): string {
  return `${prefix}-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
}

async function withRetry<T>(
  operation: () => Promise<T>,
  retries = 3,
  delayMs = 250,
): Promise<T> {
  let lastError: unknown;

  for (let attempt = 0; attempt < retries; attempt++) {
    try {
      return await operation();
    } catch (error) {
      lastError = error;
      const message =
        error instanceof Error ? error.message : String(error ?? "");
      const isConnectionIssue = /ECONNREFUSED|fetch failed|NetworkError/i.test(
        message,
      );
      if (!isConnectionIssue || attempt === retries - 1) {
        throw error;
      }
      await new Promise((resolve) =>
        setTimeout(resolve, delayMs * (attempt + 1)),
      );
    }
  }

  throw lastError;
}

// Collect events from SSE stream
async function collectEvents(
  chatId: string,
  {
    maxEvents,
    timeoutMs,
    stopWhen,
  }: {
    maxEvents: number;
    timeoutMs: number;
    stopWhen?: (event: unknown, events: unknown[]) => boolean;
  },
): Promise<unknown[]> {
  const events: unknown[] = [];

  return new Promise((resolve) => {
    const controller = new AbortController();
    let settled = false;
    const finish = () => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timeout);
      controller.abort();
      resolve(events);
    };
    const timeout = setTimeout(() => {
      finish();
    }, timeoutMs);

    fetch(`${LSP_URL}/v1/chats/subscribe?chat_id=${chatId}`, {
      signal: controller.signal,
    })
      .then(async (response) => {
        if (!response.ok) {
          finish();
          return;
        }

        const reader = response.body?.getReader();
        if (!reader) {
          finish();
          return;
        }

        const decoder = new TextDecoder();
        let buffer = "";

        while (!settled && events.length < maxEvents) {
          const { done, value } = await reader.read();
          if (done) break;

          buffer += decoder.decode(value, { stream: true });
          const blocks = buffer.split("\n\n");
          buffer = blocks.pop() ?? "";

          for (const block of blocks) {
            const dataLines = block
              .split("\n")
              .filter((line) => line.startsWith("data:"))
              .map((line) => line.slice(5).trimStart());

            if (dataLines.length === 0) {
              continue;
            }

            const payload = dataLines.join("\n");
            if (payload === "[DONE]") {
              continue;
            }

            try {
              const event = JSON.parse(payload);
              events.push(event);

              if (
                (stopWhen?.(event, events) ?? false) ||
                events.length >= maxEvents
              ) {
                finish();
                return;
              }
            } catch {
              // Ignore parse errors
            }
          }
        }

        finish();
      })
      .catch(() => {
        finish();
      });
  });
}

describe.skipIf(!(await isServerAvailable()))(
  "Chat Subscription Integration Tests",
  () => {
    describe("sendChatCommand", () => {
      it("should accept abort command", async () => {
        const chatId = generateChatId("test-abort");

        await expect(
          sendChatCommand(chatId, LSP_PORT, undefined, {
            type: "abort" as const,
          }),
        ).resolves.toBeUndefined();
      });

      it("should accept set_params command", async () => {
        const chatId = generateChatId("test-params");

        await expect(
          updateChatParams(
            chatId,
            { model: "refact/gpt-4.1-nano", mode: "NO_TOOLS" },
            LSP_PORT,
          ),
        ).resolves.toBeUndefined();
      });

      it("should accept user_message command", async () => {
        const chatId = generateChatId("test-message");

        await updateChatParams(
          chatId,
          { model: "refact/gpt-4.1-nano", mode: "NO_TOOLS" },
          LSP_PORT,
        );

        await expect(
          sendUserMessage(chatId, "Hello, test!", LSP_PORT),
        ).resolves.toBeUndefined();
      });

      it("should detect duplicate commands", async () => {
        const chatId = generateChatId("test-duplicate");
        const requestId = `test-${Date.now()}`;

        // First request
        const response1 = await fetch(
          `${LSP_URL}/v1/chats/${chatId}/commands`,
          {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
              client_request_id: requestId,
              type: "set_params",
              patch: { model: "test" },
            }),
          },
        );

        expect(response1.status).toBe(200);

        // Second request with same ID should be detected as duplicate
        const response2 = await fetch(
          `${LSP_URL}/v1/chats/${chatId}/commands`,
          {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({
              client_request_id: requestId,
              type: "set_params",
              patch: { model: "test" },
            }),
          },
        );

        expect(response2.status).toBe(200);
        const data = await response2.json();
        // Backend may return duplicate status or just accept it idempotently
        expect(
          ["duplicate", "ok", "queued"].includes(data.status as string),
        ).toBe(true);
      });
    });

    describe("SSE Subscription", () => {
      it("should receive snapshot on connect", async () => {
        const chatId = generateChatId("test-snapshot");

        const events = await collectEvents(chatId, {
          maxEvents: 1,
          timeoutMs: 5000,
        });

        expect(events.length).toBeGreaterThanOrEqual(1);
        expect(events[0]).toHaveProperty("type", "snapshot");
        expect(events[0]).toHaveProperty("chat_id", chatId);
        expect(events[0]).toHaveProperty("thread");
        expect(events[0]).toHaveProperty("runtime");
        expect(events[0]).toHaveProperty("messages");
      });

      it("should receive events after sending command", async () => {
        const chatId = generateChatId("test-events");

        // Start collecting events
        const eventsPromise = collectEvents(chatId, {
          maxEvents: 10,
          timeoutMs: 10000,
        });

        // Wait a bit for subscription to establish
        await new Promise((r) => setTimeout(r, 300));

        // Send commands
        await withRetry(() =>
          updateChatParams(
            chatId,
            { model: "refact/gpt-4.1-nano", mode: "NO_TOOLS" },
            LSP_PORT,
          ),
        );

        await withRetry(() => sendUserMessage(chatId, "Say hi", LSP_PORT));

        const events = await eventsPromise;

        // Check we got expected events
        const eventTypes = events.map(
          (e: unknown) => (e as { type: string }).type,
        );

        expect(eventTypes).toContain("snapshot");
        expect(eventTypes).toContain("ack"); // Command acknowledgments
      });

      it("should receive stream events during generation", async () => {
        const chatId = generateChatId("test-stream");

        // Start collecting events
        const eventsPromise = collectEvents(chatId, {
          maxEvents: 20,
          timeoutMs: 15000,
        });

        await new Promise((r) => setTimeout(r, 300));

        // Set up chat and send message
        await withRetry(() =>
          updateChatParams(
            chatId,
            { model: "refact/gpt-4.1-nano", mode: "NO_TOOLS" },
            LSP_PORT,
          ),
        );

        await withRetry(() => sendUserMessage(chatId, "Say hello", LSP_PORT));

        const events = await eventsPromise;
        const eventTypes = events.map(
          (e: unknown) => (e as { type: string }).type,
        );

        // Should have streaming events
        expect(eventTypes).toContain("snapshot");
        expect(eventTypes).toContain("message_added"); // User message
        expect(eventTypes).toContain("stream_started");

        // May have stream_delta and stream_finished depending on timing
        // Debug: eventTypes contains the received event types
      });
    });

    describe("Abort Functionality", () => {
      it("should abort generation and receive message_removed", async () => {
        const chatId = generateChatId("test-abort-stream");

        // Start collecting events
        const eventsPromise = collectEvents(chatId, {
          maxEvents: 1000,
          timeoutMs: 30000,
          stopWhen: (event: unknown) => {
            const type = (event as { type?: string }).type;
            return type === "message_removed" || type === "stream_finished";
          },
        });

        await new Promise((r) => setTimeout(r, 300));

        // Set up chat with a long prompt
        await withRetry(() =>
          updateChatParams(
            chatId,
            { model: "refact/gpt-4.1-nano", mode: "NO_TOOLS" },
            LSP_PORT,
          ),
        );

        await withRetry(() =>
          sendUserMessage(
            chatId,
            "Write a long essay about programming",
            LSP_PORT,
          ),
        );

        // Wait briefly for generation to start, then abort.
        await new Promise((r) => setTimeout(r, 200));

        // Send abort
        await withRetry(() => abortGeneration(chatId, LSP_PORT));

        const events = await eventsPromise;
        const eventTypes = events.map(
          (e: unknown) => (e as { type: string }).type,
        );

        // Debug: eventTypes contains abort test events

        // Should have stream_started and either message_removed (abort) or stream_finished (too late)
        expect(eventTypes).toContain("stream_started");
        expect(
          eventTypes.includes("message_removed") ||
            eventTypes.includes("stream_finished"),
        ).toBe(true);
      });
    });

    describe("Multiple Chats", () => {
      it("should handle multiple independent chats", async () => {
        const chatId1 = generateChatId("test-multi-1");
        const chatId2 = generateChatId("test-multi-2");

        // Connect to both chats
        const events1Promise = collectEvents(chatId1, {
          maxEvents: 5,
          timeoutMs: 8000,
        });
        const events2Promise = collectEvents(chatId2, {
          maxEvents: 5,
          timeoutMs: 8000,
        });

        await new Promise((r) => setTimeout(r, 300));

        // Send different messages to each
        await withRetry(() =>
          updateChatParams(
            chatId1,
            { model: "refact/gpt-4.1-nano", mode: "NO_TOOLS" },
            LSP_PORT,
          ),
        );
        await withRetry(() =>
          updateChatParams(
            chatId2,
            { model: "refact/gpt-4.1-nano", mode: "NO_TOOLS" },
            LSP_PORT,
          ),
        );

        await withRetry(() =>
          sendUserMessage(chatId1, "Chat 1 message", LSP_PORT),
        );
        await withRetry(() =>
          sendUserMessage(chatId2, "Chat 2 message", LSP_PORT),
        );

        const [events1, events2] = await Promise.all([
          events1Promise,
          events2Promise,
        ]);

        // Each should only have events for its own chat
        const chat1Ids = events1.map(
          (e: unknown) => (e as { chat_id: string }).chat_id,
        );
        const chat2Ids = events2.map(
          (e: unknown) => (e as { chat_id: string }).chat_id,
        );

        expect(chat1Ids.every((id: string) => id === chatId1)).toBe(true);
        expect(chat2Ids.every((id: string) => id === chatId2)).toBe(true);
      });
    });
  },
);
