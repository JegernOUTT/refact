import { http, HttpResponse } from "msw";
import { describe, expect, test } from "vitest";

import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
  waitFor,
} from "../../../utils/test-utils";
import { server } from "../../../utils/mockServer";
import type { ToolCall, ToolMessage } from "../../../services/refact/types";
import { ExecToolCard } from "./ExecToolCard";

const receivedChars: string[] = [];

function toolCall(): ToolCall {
  return {
    id: "exec-call",
    index: 0,
    type: "function",
    function: {
      name: "shell",
      arguments: JSON.stringify({ command: "python" }),
    },
  };
}

function toolMessage(tty: boolean): ToolMessage {
  return {
    role: "tool",
    content: "",
    tool_call_id: "exec-call",
    extra: {
      exec: {
        process_id: "exec_test",
        status: "running",
        short_description: "python",
        command: "python",
        tty,
      },
    },
  };
}

async function renderExpandedStdinCard(tty: boolean) {
  const chat = createDefaultChatState();
  const currentThread = chat.threads[chat.current_thread_id];
  currentThread.thread.messages = [toolMessage(tty)];
  const result = render(
    <ExecToolCard toolCall={toolCall()} toolName="shell" />,
    {
      preloadedState: { chat },
    },
  );
  await result.user.click(screen.getByText("python"));
  return result;
}

describe("ProcessStdinInput", () => {
  test("renders when tty is true", async () => {
    await renderExpandedStdinCard(true);

    expect(screen.getByText("Send Ctrl+C")).toBeInTheDocument();
    expect(
      screen.getByText("Interactive process — direct stdin available"),
    ).toBeInTheDocument();
  });

  test("hides when tty is false", async () => {
    await renderExpandedStdinCard(false);

    expect(screen.queryByText("Send Ctrl+C")).toBeNull();
    expect(
      screen.queryByText("Interactive process — direct stdin available"),
    ).toBeNull();
  });

  test("submit calls API and clears the input", async () => {
    receivedChars.length = 0;
    server.use(
      http.post("*/v1/exec/:processId/stdin", async ({ request }) => {
        const body = (await request.json()) as { chars: string };
        receivedChars.push(body.chars);
        return HttpResponse.json({
          process_id: "exec_test",
          status: "running",
          bytes_written: body.chars.length,
          since_seq: 0,
          next_seq: 0,
          latest_seq: 0,
        });
      }),
    );

    await renderExpandedStdinCard(true);

    const input = screen.getByLabelText<HTMLInputElement>("Process stdin");
    fireEvent.change(input, { target: { value: "hello" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(receivedChars).toEqual(["hello"]));
    await waitFor(() => expect(input.value).toBe(""));
  });
});
