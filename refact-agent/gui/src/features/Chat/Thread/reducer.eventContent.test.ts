/* eslint-disable @typescript-eslint/no-non-null-assertion */
import { beforeEach, describe, expect, test } from "vitest";
import { chatReducer } from "./reducer";
import type { Chat } from "./types";
import { newChatAction, applyChatEvent } from "./actions";
import type { ChatMessages } from "../../../services/refact/types";
import {
  buildDisplayItems,
  nativeToolJobResult,
  tryIncrementalDisplayItemsUpdate,
} from "../../../components/ChatContent/ChatContentDisplayItems";

const IMAGE = "c3ludGhldGlj";

function wireCompletion(overrides: Record<string, unknown> = {}) {
  return {
    role: "event",
    message_id: "tool_job:job-1:1:0",
    content: [
      { m_type: "text", m_content: "Synthetic capture" },
      { m_type: "image/png", m_content: IMAGE },
    ],
    event: {
      subkind: "system_notice",
      source: "tool_job.completed",
      payload: { actor: "tool_completion", job_id: "job-1" },
    },
    tool_job_result: {
      actor: "tool_completion",
      job_id: "job-1",
      revision: 1,
      tool_name: "app_screenshot",
      tool_call_id: "call-1",
      original_role: "tool",
    },
    delivery: { source: "tool_job.completed", id: "tool_job:job-1:1" },
    ...overrides,
  };
}

const assistantCall = {
  role: "assistant",
  content: "",
  message_id: "assistant-1",
  tool_calls: [
    {
      id: "call-1",
      index: 0,
      type: "function",
      function: { name: "app_screenshot", arguments: '{"action":"capture"}' },
    },
  ],
};

describe("event content wire normalization", () => {
  let state: Chat;
  let chatId: string;

  beforeEach(() => {
    const empty = chatReducer(undefined, { type: "@@INIT" });
    state = chatReducer(empty, newChatAction(undefined));
    chatId = state.current_thread_id;
  });

  function add(message: unknown, seq: string): ChatMessages {
    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq,
        type: "message_added",
        index: Number(seq) - 1,
        message: message as ChatMessages[number],
      }),
    );
    return state.threads[chatId]!.thread.messages;
  }

  test("lifts flattened completion metadata into extra and keeps multimodal content", () => {
    add(assistantCall, "1");
    const messages = add(
      wireCompletion({
        tool_failed: true,
        privacy_shell: { visibility: "local_only" },
      }),
      "2",
    );
    const stored = messages[1] as unknown as Record<string, unknown>;
    expect(stored.tool_job_result).toBeUndefined();
    expect(stored.delivery).toBeUndefined();
    expect(stored.privacy_shell).toBeUndefined();
    expect(stored.extra).toMatchObject({
      tool_job_result: { job_id: "job-1" },
      delivery: { id: "tool_job:job-1:1" },
      privacy_shell: { visibility: "local_only" },
    });
    expect(stored.content).toEqual(wireCompletion().content);

    const items = buildDisplayItems(messages, false);
    const result = items.find((item) => item.type === "tool_job_result");
    expect(result).toMatchObject({
      type: "tool_job_result",
      result: {
        toolName: "app_screenshot",
        toolCall: { id: "call-1" },
        message: {
          role: "tool",
          tool_call_id: "call-1",
          tool_failed: true,
          content: wireCompletion().content,
        },
      },
    });
  });

  test("direct flattened wire keeps privacy metadata on the native view", () => {
    const wire = wireCompletion({
      privacy_shell: { visibility: "local_only" },
      extra: { privacy_shell: { visibility: "nested" } },
    }) as unknown as ChatMessages[number];
    const result = nativeToolJobResult(wire);
    expect(result?.message.extra).toMatchObject({
      privacy_shell: { visibility: "nested" },
      tool_job_result: { job_id: "job-1" },
    });
    const flatOnly = wireCompletion({
      privacy_shell: { visibility: "local_only" },
    }) as unknown as ChatMessages[number];
    expect(nativeToolJobResult(flatOnly)?.message.extra).toMatchObject({
      privacy_shell: { visibility: "local_only" },
    });
  });

  test("resolves the nearest prior call and rejects a conflicting rename", () => {
    add(assistantCall, "1");
    add(
      {
        ...assistantCall,
        message_id: "assistant-2",
        tool_calls: [
          {
            ...assistantCall.tool_calls[0],
            function: { name: "other_tool", arguments: "{}" },
          },
        ],
      },
      "2",
    );
    const messages = add(wireCompletion(), "3");
    const result = buildDisplayItems(messages, false).find(
      (item) => item.type === "tool_job_result",
    );
    expect(result?.type).toBe("tool_job_result");
    if (result?.type === "tool_job_result") {
      expect(result.result.toolCall).toBeUndefined();
    }
  });

  test("existing extra metadata wins over flattened duplicates", () => {
    const messages = add(
      wireCompletion({
        privacy_shell: { visibility: "wire" },
        extra: { privacy_shell: { visibility: "extra" } },
      }),
      "1",
    );
    expect(messages[0].extra?.privacy_shell).toEqual({ visibility: "extra" });
  });

  test("does not attach a mismatched original tool call", () => {
    add(
      {
        ...assistantCall,
        tool_calls: [
          {
            ...assistantCall.tool_calls[0],
            function: { name: "other_tool", arguments: "{}" },
          },
        ],
      },
      "1",
    );
    const messages = add(wireCompletion(), "2");
    const result = buildDisplayItems(messages, false).find(
      (item) => item.type === "tool_job_result",
    );
    expect(result?.type).toBe("tool_job_result");
    if (result?.type === "tool_job_result") {
      expect(result.result.toolCall).toBeUndefined();
    }
  });

  test("refreshes linked App call arguments after an incremental message update", () => {
    add(assistantCall, "1");
    const previous = add(wireCompletion(), "2");
    const previousItems = buildDisplayItems(previous, false);
    const updatedCall = {
      ...assistantCall,
      tool_calls: [
        {
          ...assistantCall.tool_calls[0],
          function: {
            name: "app_screenshot",
            arguments: '{"action":"capture","view_id":"new-view"}',
          },
        },
      ],
    };
    state = chatReducer(
      state,
      applyChatEvent({
        chat_id: chatId,
        seq: "3",
        type: "message_updated",
        message_id: "assistant-1",
        message: updatedCall as ChatMessages[number],
      }),
    );
    const next = state.threads[chatId]!.thread.messages;
    const incremental = tryIncrementalDisplayItemsUpdate(
      previous,
      next,
      previousItems,
      false,
    );
    expect(incremental).not.toBeNull();
    expect(incremental).toEqual(buildDisplayItems(next, false));
    const result = incremental?.find((item) => item.type === "tool_job_result");
    expect(result).toMatchObject({
      result: { toolCall: updatedCall.tool_calls[0] },
    });
  });

  test.each([
    [
      "text and image",
      [
        { m_type: "text", m_content: "Notice text" },
        { m_type: "image/png", m_content: IMAGE },
      ],
    ],
    ["image only", [{ m_type: "image/png", m_content: IMAGE }]],
  ])(
    "keeps a multimodal %s notice as a visible event without metadata",
    (_label, content) => {
      const messages = add(
        {
          role: "event",
          message_id: "event-1",
          content,
          event: {
            subkind: "system_notice",
            source: "tool_job.completed",
            payload: {},
          },
        },
        "1",
      );
      const items = buildDisplayItems(messages, false);
      expect(items).toHaveLength(1);
      expect(items[0]).toMatchObject({ type: "event", event: { content } });
    },
  );

  test("leaves ordinary text events unchanged", () => {
    const message = {
      role: "event",
      message_id: "event-2",
      content: "Plain notice",
      event: { subkind: "system_notice", source: "chat.session" },
    };
    const messages = add(message, "1");
    expect(messages[0]).toEqual(message);
    expect(buildDisplayItems(messages, false)[0]).toMatchObject({
      type: "event",
      event: { content: "Plain notice" },
    });
  });
});
