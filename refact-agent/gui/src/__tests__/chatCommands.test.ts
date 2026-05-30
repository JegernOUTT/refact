import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  sendChatCommand,
  sendUserMessage,
  updateChatParams,
  abortGeneration,
  respondToToolConfirmation,
  respondToToolConfirmations,
  updateMessage,
  removeMessage,
  cancelQueuedItem,
  normalizeConnection,
  type ChatCommand,
} from "../services/refact/chatCommands";
import type { EngineApiConfig } from "../services/refact/apiUrl";

type MockRequestInit = { body?: string; headers?: Record<string, string> };
type MockCall = [string, MockRequestInit];

const mockFetch =
  vi.fn<(url: string, init: MockRequestInit) => Promise<Response>>();

function getRequestBody(call: MockCall): Record<string, unknown> {
  return JSON.parse(call[1].body ?? "{}") as Record<string, unknown>;
}

function getRequestUrl(): string {
  return (mockFetch.mock.calls[0] as MockCall)[0];
}

describe("chatCommands", () => {
  beforeEach(() => {
    global.fetch = mockFetch as unknown as typeof fetch;
    mockFetch.mockReset();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("normalizeConnection", () => {
    it("maps numeric legacy input to local IDE mode", () => {
      expect(normalizeConnection(8123)).toEqual({ host: "ide", lspPort: 8123 });
    });
  });

  describe("sendChatCommand", () => {
    it("uses Vite relative command URL in relative mode", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await sendChatCommand(
        "chat/with spaces",
        { host: "web", dev: true, lspPort: 8001 },
        undefined,
        { type: "abort" },
      );

      expect(mockFetch).toHaveBeenCalledWith(
        "/v1/chats/chat%2Fwith%20spaces/commands",
        expect.objectContaining({
          method: "POST",
          headers: { "Content-Type": "application/json" },
        }),
      );
    });

    it("uses engine-served relative command URL", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await sendChatCommand(
        "engine-chat",
        { host: "web", engineServed: true, lspUrl: "https://ignored.test" },
        undefined,
        { type: "abort" },
      );

      expect(getRequestUrl()).toBe("/v1/chats/engine-chat/commands");
    });

    it("uses configured remote origin in remote web mode", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await sendChatCommand(
        "remote-chat",
        {
          host: "web",
          lspUrl: "https://engine.example.com/proxy/v1/ping/Refact?stale=1",
          lspPort: 0,
        },
        undefined,
        { type: "abort" },
      );

      expect(getRequestUrl()).toBe(
        "https://engine.example.com/proxy/v1/chats/remote-chat/commands",
      );
    });

    it("keeps numeric legacy local fallback for compatibility", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      const chatId = "test-chat-123";
      const port = 8001;
      const command = { type: "abort" as const };

      await sendChatCommand(chatId, port, undefined, command);

      expect(mockFetch).toHaveBeenCalledWith(
        `http://127.0.0.1:${port}/v1/chats/${chatId}/commands`,
        expect.objectContaining({
          method: "POST",
          headers: { "Content-Type": "application/json" },
        }),
      );
    });

    it("includes client_request_id in request body", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await sendChatCommand("test", 8001, undefined, { type: "abort" });

      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody).toHaveProperty("client_request_id");
      expect(typeof calledBody.client_request_id).toBe("string");
      expect(calledBody.type).toBe("abort");
    });

    it("includes authorization header when apiKey provided", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await sendChatCommand("test", 8001, "test-key", {
        type: "abort",
      });

      const call = mockFetch.mock.calls[0] as MockCall;
      expect(call[1].headers).toHaveProperty(
        "Authorization",
        "Bearer test-key",
      );
    });

    it("throws on HTTP error", async () => {
      mockFetch.mockResolvedValueOnce({
        ok: false,
        status: 500,
        statusText: "Internal Server Error",
        text: () => Promise.resolve("Error details"),
      } as Response);

      await expect(
        sendChatCommand("test", 8001, undefined, { type: "abort" }),
      ).rejects.toThrow("Failed to send command");
    });
  });

  describe("cancelQueuedItem", () => {
    it("uses configured base and encodes queue cancellation IDs", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      const result = await cancelQueuedItem(
        "chat/id",
        "request/id with spaces",
        { host: "web", dev: true },
        "test-key",
      );

      expect(result).toBe(true);
      expect(mockFetch).toHaveBeenCalledWith(
        "/v1/chats/chat%2Fid/queue/request%2Fid%20with%20spaces",
        {
          method: "DELETE",
          headers: { Authorization: "Bearer test-key" },
        },
      );
    });

    it("uses remote URL for queue cancellation", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await cancelQueuedItem("chat", "request", {
        host: "web",
        lspUrl: "https://remote.example.com/base",
      });

      expect(getRequestUrl()).toBe(
        "https://remote.example.com/base/v1/chats/chat/queue/request",
      );
    });
  });

  describe("sendUserMessage", () => {
    it("sends user_message command with string content", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await sendUserMessage("test-chat", "Hello world", 8001);

      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody.type).toBe("user_message");
      expect(calledBody.content).toBe("Hello world");
    });

    it("sends user_message command with multi-modal content", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      const content = [
        { type: "text" as const, text: "What is this?" },
        {
          type: "image_url" as const,
          image_url: { url: "data:image/png;base64,..." },
        },
      ];

      await sendUserMessage("test-chat", content, 8001);

      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody.type).toBe("user_message");
      expect(Array.isArray(calledBody.content)).toBe(true);
      expect(calledBody.content).toEqual(content);
    });
  });

  describe("updateChatParams", () => {
    it("sends set_params command", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await updateChatParams(
        "test-chat",
        { model: "gpt-4", mode: "AGENT" },
        8001,
      );

      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody.type).toBe("set_params");
      expect(calledBody.patch).toEqual({ model: "gpt-4", mode: "AGENT" });
    });

    it("accepts full EngineApiConfig for params updates", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);
      const config: EngineApiConfig = { host: "web", dev: true };

      await updateChatParams("test-chat", { boost_reasoning: true }, config);

      expect(getRequestUrl()).toBe("/v1/chats/test-chat/commands");
      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody.type).toBe("set_params");
      expect(calledBody.patch).toEqual({ boost_reasoning: true });
    });
  });

  describe("abortGeneration", () => {
    it("sends abort command", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await abortGeneration("test-chat", 8001);

      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody.type).toBe("abort");
    });
  });

  describe("respondToToolConfirmation", () => {
    it("sends tool_decision command with accepted=true", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await respondToToolConfirmation("test-chat", "call_123", true, 8001);

      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody.type).toBe("tool_decision");
      expect(calledBody.tool_call_id).toBe("call_123");
      expect(calledBody.accepted).toBe(true);
    });

    it("sends tool_decision command with accepted=false", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await respondToToolConfirmation("test-chat", "call_456", false, 8001);

      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody.type).toBe("tool_decision");
      expect(calledBody.tool_call_id).toBe("call_456");
      expect(calledBody.accepted).toBe(false);
    });
  });

  describe("respondToToolConfirmations", () => {
    it("sends tool_decisions command with object array", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      const decisions = [
        { tool_call_id: "call_1", accepted: true },
        { tool_call_id: "call_2", accepted: false },
        { tool_call_id: "call_3", accepted: true },
      ];

      await respondToToolConfirmations("test-chat", decisions, 8001);

      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody.type).toBe("tool_decisions");
      expect(calledBody.decisions).toEqual(decisions);
    });
  });

  describe("updateMessage", () => {
    it("sends update_message command", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await updateMessage("test-chat", "msg_5", "Updated text", 8001);

      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody.type).toBe("update_message");
      expect(calledBody.message_id).toBe("msg_5");
      expect(calledBody.content).toBe("Updated text");
    });

    it("sends update_message with regenerate flag", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await updateMessage(
        "test-chat",
        "msg_5",
        "Updated text",
        8001,
        undefined,
        true,
      );

      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody.type).toBe("update_message");
      expect(calledBody.regenerate).toBe(true);
    });
  });

  describe("removeMessage", () => {
    it("sends remove_message command", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await removeMessage("test-chat", "msg_5", 8001);

      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody.type).toBe("remove_message");
      expect(calledBody.message_id).toBe("msg_5");
    });

    it("sends remove_message with regenerate flag", async () => {
      mockFetch.mockResolvedValueOnce({ ok: true } as Response);

      await removeMessage("test-chat", "msg_5", 8001, undefined, true);

      const calledBody = getRequestBody(mockFetch.mock.calls[0] as MockCall);
      expect(calledBody.type).toBe("remove_message");
      expect(calledBody.regenerate).toBe(true);
    });
  });
});

describe("Command Types", () => {
  it("correctly types user_message command with string", () => {
    const command: ChatCommand = {
      type: "user_message",
      content: "Hello",
      attachments: [],
      client_request_id: "test-id",
    };

    expect(command.type).toBe("user_message");
  });

  it("correctly types user_message command with multimodal array", () => {
    const command: ChatCommand = {
      type: "user_message",
      content: [
        { type: "text", text: "Hello" },
        { type: "image_url", image_url: { url: "data:..." } },
      ],
      attachments: [],
      client_request_id: "test-id",
    };

    expect(command.type).toBe("user_message");
  });

  it("correctly types set_params command", () => {
    const command: ChatCommand = {
      type: "set_params",
      patch: {
        model: "gpt-4",
        mode: "AGENT",
        boost_reasoning: true,
      },
      client_request_id: "test-id",
    };

    expect(command.type).toBe("set_params");
  });

  it("correctly types abort command", () => {
    const command: ChatCommand = {
      type: "abort",
      client_request_id: "test-id",
    };
    expect(command.type).toBe("abort");
  });

  it("correctly types tool_decision command", () => {
    const command: ChatCommand = {
      type: "tool_decision",
      tool_call_id: "call_123",
      accepted: true,
      client_request_id: "test-id",
    };

    expect(command.type).toBe("tool_decision");
  });

  it("correctly types ide_tool_result command", () => {
    const command: ChatCommand = {
      type: "ide_tool_result",
      tool_call_id: "call_123",
      content: "result",
      tool_failed: false,
      client_request_id: "test-id",
    };

    expect(command.type).toBe("ide_tool_result");
  });

  it("correctly types tool_decisions command", () => {
    const command: ChatCommand = {
      type: "tool_decisions",
      decisions: [
        { tool_call_id: "call_1", accepted: true },
        { tool_call_id: "call_2", accepted: false },
      ],
      client_request_id: "test-id",
    };

    expect(command.type).toBe("tool_decisions");
  });

  it("correctly types update_message command", () => {
    const command: ChatCommand = {
      type: "update_message",
      message_id: "msg_5",
      content: "Updated",
      regenerate: true,
      client_request_id: "test-id",
    };

    expect(command.type).toBe("update_message");
  });

  it("correctly types remove_message command", () => {
    const command: ChatCommand = {
      type: "remove_message",
      message_id: "msg_5",
      regenerate: false,
      client_request_id: "test-id",
    };

    expect(command.type).toBe("remove_message");
  });
});
