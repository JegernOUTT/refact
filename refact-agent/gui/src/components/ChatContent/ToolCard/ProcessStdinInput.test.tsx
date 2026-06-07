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
import type { Config } from "../../../features/Config/configSlice";

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

async function renderExpandedStdinCard(tty: boolean, config?: Partial<Config>) {
  const chat = createDefaultChatState();
  const currentThread = chat.threads[chat.current_thread_id];
  currentThread.thread.messages = [toolMessage(tty)];
  const result = render(
    <ExecToolCard toolCall={toolCall()} toolName="shell" />,
    {
      preloadedState: config ? { chat, config: config as Config } : { chat },
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

    await renderExpandedStdinCard(true, { host: "vscode", lspPort: 8001 });

    const input = screen.getByLabelText<HTMLInputElement>("Process stdin");
    fireEvent.change(input, { target: { value: "hello" } });
    fireEvent.click(screen.getByRole("button", { name: "Send" }));

    await waitFor(() => expect(receivedChars).toEqual(["hello"]));
    await waitFor(() => expect(input.value).toBe(""));
  });

  test("submits with remote lspUrl when lspPort is zero", async () => {
    receivedChars.length = 0;
    let receivedUrl = "";
    server.use(
      http.post(
        "https://engine.example.test/v1/exec/:processId/stdin",
        async ({ request }) => {
          receivedUrl = request.url;
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
        },
      ),
    );

    await renderExpandedStdinCard(true, {
      host: "vscode",
      lspPort: 0,
      lspUrl: "https://engine.example.test",
    });

    const input = screen.getByLabelText<HTMLInputElement>("Process stdin");
    expect(input).toBeEnabled();
    const sendCtrlC = screen.getByRole("button", { name: "Send Ctrl+C" });
    expect(sendCtrlC).toBeEnabled();
    fireEvent.change(input, { target: { value: "remote" } });
    const send = screen.getByRole("button", { name: "Send" });
    expect(send).toBeEnabled();
    fireEvent.click(send);

    await waitFor(() => expect(receivedChars).toEqual(["remote"]));
    expect(receivedUrl).toBe(
      "https://engine.example.test/v1/exec/exec_test/stdin",
    );
  });
});
