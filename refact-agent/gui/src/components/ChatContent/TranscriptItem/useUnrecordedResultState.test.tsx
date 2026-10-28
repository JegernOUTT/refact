import { describe, expect, it } from "vitest";
import React from "react";
import {
  createDefaultChatState,
  render,
  screen,
} from "../../../utils/test-utils";
import type { ToolCall } from "../../../services/refact/types";
import { useUnrecordedResultState } from "./useUnrecordedResultState";
import { NEEDS_APPROVAL_LABEL, NO_RESULT_LABEL } from "./unrecordedResult";

function call(id: string): ToolCall {
  return {
    id,
    index: 0,
    type: "function",
    function: { name: "unknown_tool", arguments: "{}" },
  };
}

const Probe: React.FC<{ toolCallId: string }> = ({ toolCallId }) => {
  const state = useUnrecordedResultState(toolCallId);
  return (
    <span data-testid={`probe-${toolCallId}`}>
      {`${state.status}|${state.statusLabel ?? ""}`}
    </span>
  );
};

function renderProbes(options: { busy: boolean; pausedCallId?: string }) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.streaming = options.busy;
  runtime.thread.messages = [
    { role: "assistant", content: "", tool_calls: [call("stale")] },
    { role: "assistant", content: "", tool_calls: [call("latest")] },
  ];
  if (options.pausedCallId) {
    runtime.confirmation = {
      ...runtime.confirmation,
      pause: true,
      pause_reasons: [
        {
          type: "confirmation",
          tool_name: "unknown_tool",
          command: "unknown_tool",
          rule: "ask",
          tool_call_id: options.pausedCallId,
          integr_config_path: null,
        },
      ],
    };
  }
  render(
    <>
      <Probe toolCallId="stale" />
      <Probe toolCallId="latest" />
    </>,
    { preloadedState: { chat } },
  );
}

describe("useUnrecordedResultState", () => {
  it("only the latest assistant tool-call group runs while the thread is busy", () => {
    renderProbes({ busy: true });
    expect(screen.getByTestId("probe-stale")).toHaveTextContent(
      `unknown|${NO_RESULT_LABEL}`,
    );
    expect(screen.getByTestId("probe-latest")).toHaveTextContent("running|");
  });

  it("idle threads report no result for every unresolved call", () => {
    renderProbes({ busy: false });
    expect(screen.getByTestId("probe-stale")).toHaveTextContent(
      `unknown|${NO_RESULT_LABEL}`,
    );
    expect(screen.getByTestId("probe-latest")).toHaveTextContent(
      `unknown|${NO_RESULT_LABEL}`,
    );
  });

  it.each([true, false])(
    "pause reasons take precedence over busy=%s and stale ownership",
    (busy) => {
      renderProbes({ busy, pausedCallId: "stale" });
      expect(screen.getByTestId("probe-stale")).toHaveTextContent(
        `needs_you|${NEEDS_APPROVAL_LABEL}`,
      );
    },
  );
});
