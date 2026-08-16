import { HttpResponse, http } from "msw";
import { describe, expect, it, vi } from "vitest";

import {
  createDefaultChatState,
  render,
  screen,
  waitFor,
} from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import type {
  AssistantMessage,
  ChatMessages,
  ToolCall,
  ToolMessage,
} from "../../services/refact/types";
import type {
  PrivacyDestination,
  PrivacyPolicyResponse,
} from "../../services/refact/privacy";
import { ChatShield } from "./ChatShield";
import { BlockCard } from "./BlockCard";
import { WithheldOutputCard } from "./WithheldOutputCard";
import { ToolContent } from "../../components/ChatContent/ToolsContent";

const destination: PrivacyDestination = {
  id: "untrusted",
  kind: "provider",
  display_name: "untrusted/model",
};

const policy: PrivacyPolicyResponse = {
  policy: {
    blocked: [],
    zones: [
      {
        name: "secrets",
        patterns: [".env"],
        send_to: ["trusted"],
        on_shell_read: "withhold",
      },
      {
        name: "normal",
        patterns: ["**"],
        send_to: ["*"],
        on_shell_read: "withhold",
      },
    ],
    subagents: { report_declassifies: true },
  },
  destinations: [destination],
  match_counts: { secrets: 1, normal: 4 },
  error: null,
  source_paths: [],
};

const file = {
  path: ".env",
  zone: "secrets",
  attribution: "observed" as const,
};

function installPolicyHandler() {
  server.use(http.get("*/v1/privacy/policy", () => HttpResponse.json(policy)));
}

function chatWithMessages(messages: ChatMessages) {
  const chat = createDefaultChatState();
  const runtime = chat.threads[chat.current_thread_id];
  runtime.thread.model = destination.display_name;
  runtime.thread.messages = messages;
  return chat;
}

function toolCall(id = "call-shell"): ToolCall {
  return {
    id,
    index: 0,
    type: "function",
    function: {
      name: "shell",
      arguments: JSON.stringify({ command: "cat .env" }),
    },
  };
}

describe("Privacy shield", () => {
  it("shows the current model, restricted count, and destination inspection", async () => {
    installPolicyHandler();
    server.use(
      http.post("*/v1/privacy/inspect", () =>
        HttpResponse.json({
          chat_id: "chat",
          destination,
          sendable: false,
          would_send: [],
          records: [file],
          blocked: [{ record_index: 0, record: file }],
          refusal: "destination untrusted cannot receive message 0 path .env",
        }),
      ),
    );
    const chat = chatWithMessages([
      {
        role: "tool",
        tool_call_id: "call-shell",
        content: "withheld",
        extra: { privacy: { files: [file] } },
      },
    ] as ChatMessages);

    const { user } = render(<ChatShield threadId={chat.current_thread_id} />, {
      preloadedState: { chat },
    });

    expect(
      await screen.findByText("1 thing here can't go to untrusted/model"),
    ).toBeVisible();
    await user.click(
      screen.getByRole("button", { name: "Inspect destination" }),
    );

    expect(
      await screen.findByRole("heading", { name: "Destination inspector" }),
    ).toBeVisible();
    expect(
      await screen.findByText("1 records cannot go to this model"),
    ).toBeVisible();
    expect(screen.getByText(".env")).toBeVisible();
  });
});

describe("Privacy block card actions", () => {
  it("offers exactly switch-model and clean-branch recovery actions", async () => {
    const onSwitchModel = vi.fn();
    const onBranchCleanChat = vi.fn();
    const { user } = render(
      <BlockCard
        model="untrusted/model"
        step={3}
        blockedFiles={[file]}
        onSwitchModel={onSwitchModel}
        onBranchCleanChat={onBranchCleanChat}
      />,
    );

    const card = screen.getByTestId("privacy-block-card");
    const buttons = withinCardButtons(card);
    expect(buttons).toHaveLength(2);
    expect(card).toHaveTextContent("This model cannot receive this step");

    await user.click(
      screen.getByRole("button", { name: "Switch to an allowed model" }),
    );
    await user.click(
      screen.getByRole("button", {
        name: "Branch clean chat from before step 3",
      }),
    );
    expect(onSwitchModel).toHaveBeenCalledOnce();
    expect(onBranchCleanChat).toHaveBeenCalledOnce();
  });
});

function withinCardButtons(card: HTMLElement): HTMLButtonElement[] {
  return Array.from(card.querySelectorAll("button"));
}

describe("Privacy withheld output card", () => {
  it("reveals the existing local output without making a network request", async () => {
    const networkCalls: string[] = [];
    const requestListener = ({ request }: { request: Request }) => {
      networkCalls.push(request.url);
    };
    server.events.on("request:start", requestListener);
    const { user } = render(
      <WithheldOutputCard
        exitCode={0}
        files={[file]}
        localOnlyOutput="TOKEN=local-secret"
      />,
    );

    expect(
      screen.getByTestId("privacy-withheld-output-card"),
    ).toHaveTextContent("Ran, exit 0 — output withheld, it read .env.");
    expect(
      screen.getByTestId("privacy-withheld-output-card"),
    ).toHaveTextContent("it read .env");
    expect(screen.queryByText("TOKEN=local-secret")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Show me" }));

    expect(screen.getByText("TOKEN=local-secret")).toBeVisible();
    expect(networkCalls).toEqual([]);
    server.events.removeListener("request:start", requestListener);
  });

  it("routes withheld shell results away from the ordinary exec output", async () => {
    installPolicyHandler();
    const assistant: AssistantMessage = {
      role: "assistant",
      message_id: "assistant-before",
      content: null,
      tool_calls: [toolCall()],
    };
    const tool: ToolMessage = {
      role: "tool",
      tool_call_id: "call-shell",
      content:
        "Output withheld by user privacy policy — this command read guarded files. Other tools will refuse identically. Do not retry.",
      extra: {
        exec: {
          process_id: "exec-private",
          status: "exited",
          exit_code: 0,
        },
        privacy: { files: [file] },
        privacy_shell: {
          withheld: true,
          local_only_output: "TOKEN=local-secret",
        },
      },
    };
    const chat = chatWithMessages([assistant, tool]);
    render(<ToolContent toolCalls={[toolCall()]} />, {
      preloadedState: { chat },
    });

    expect(
      await screen.findByTestId("privacy-withheld-output-card"),
    ).toBeVisible();
    expect(screen.queryByTestId("exec-tool-card")).not.toBeInTheDocument();
    await waitFor(() => {
      expect(
        screen.getByText("Ran, exit 0 — output withheld, it read .env."),
      ).toBeVisible();
    });
  });
});
