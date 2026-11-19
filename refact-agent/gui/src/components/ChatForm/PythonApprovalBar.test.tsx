vi.hoisted(() => vi.resetModules());
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "../../utils/test-utils";
import { act } from "react-dom/test-utils";
import {
  APPROVAL_SWAP_GUARD_MS,
  PROTECTED_HINT,
  PythonApprovalBar,
  RUN_PERMIT_HINT,
} from "./PythonApprovalBar";
import { pythonApproval } from "../../__tests__/pythonFixtures";
import {
  PATCH_LONG_LINE,
  chatsPatchRequest,
  engineRequest,
} from "../ChatContent/ToolCard/engineFixtures";
import {
  UNREADABLE_ENGINE_WARNING,
  normalizePythonApprovalRequest,
} from "../../services/refact/types";
import type {
  ChatMessages,
  PythonApprovalRequest,
} from "../../services/refact/types";
import { act as rtlAct } from "@testing-library/react";
const asyncAct = rtlAct as unknown as (
  callback: () => Promise<void>,
) => Promise<void>;
import {
  ChatCommandHttpError,
  sendPythonApprovalDecision,
} from "../../services/refact/chatCommands";

vi.mock("../../services/refact/chatCommands", async (original) => ({
  ...(await original<typeof import("../../services/refact/chatCommands")>()),
  sendPythonApprovalDecision: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../../features/Chat/Thread/selectors", async (original) => ({
  ...(await original<typeof import("../../features/Chat/Thread/selectors")>()),
  selectMessagesById: () => approvals.messages,
  selectPythonApprovals: () =>
    approvals.list ??
    (approvals.pending
      ? [{ ...pythonApproval, editing_tool: approvals.editing }]
      : []),
}));

const approvals = vi.hoisted(() => ({
  pending: true,
  editing: false,
  list: undefined as PythonApprovalRequest[] | undefined,
  messages: [] as ChatMessages,
}));

describe("PythonApprovalBar", () => {
  beforeEach(() => {
    approvals.pending = true;
    approvals.editing = false;
    approvals.list = undefined;
    approvals.messages = [];
    vi.mocked(sendPythonApprovalDecision)
      .mockReset()
      .mockResolvedValue(undefined);
  });
  it("is the only action surface and locks while a decision is in flight", async () => {
    let resolve: () => void = () => undefined;
    vi.mocked(sendPythonApprovalDecision).mockImplementationOnce(
      () =>
        new Promise<void>((done) => {
          resolve = done;
        }),
    );
    render(<PythonApprovalBar chatId="chat" />);
    const button = screen.getByRole("button", { name: "Allow once" });
    fireEvent.click(button);
    expect(button).toBeDisabled();
    fireEvent.click(button);
    expect(sendPythonApprovalDecision).toHaveBeenCalledTimes(1);
    await asyncAct(async () => {
      resolve();
      await Promise.resolve();
    });
    await waitFor(() => expect(button).toBeDisabled());
  });
  it("quotes the model-written run title, never Python", () => {
    approvals.messages = [
      {
        role: "assistant",
        content: "",
        tool_calls: [
          {
            id: "t",
            index: 0,
            type: "function",
            function: {
              name: "python",
              arguments: JSON.stringify({
                code: "await shell(command='ls')",
                description: "List the files",
              }),
            },
          },
        ],
      },
    ];
    const { container } = render(<PythonApprovalBar chatId="chat" />);
    expect(screen.getByText("In run: “List the files”")).toBeInTheDocument();
    expect(container.textContent).not.toMatch(/python/i);
  });
  it("omits the run line when the script call is not found", () => {
    render(<PythonApprovalBar chatId="chat" />);
    expect(screen.queryByText(/^In run:/)).toBeNull();
  });
  it("shows the oldest request with a count of the others", () => {
    approvals.list = [
      { ...pythonApproval, request_id: "q1", tool_name: "shell" },
      { ...pythonApproval, request_id: "q2", tool_name: "cat" },
      { ...pythonApproval, request_id: "q3", tool_name: "cat" },
    ];
    render(<PythonApprovalBar chatId="chat" />);
    expect(screen.getAllByRole("button", { name: "Allow once" })).toHaveLength(
      1,
    );
    expect(screen.getByText("+2 more waiting")).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Script approval for shell" }),
    ).toBeInTheDocument();
  });
  it("titles the prompt and shows a command tool's command first", () => {
    approvals.list = [
      {
        ...pythonApproval,
        tool_name: "shell",
        command: null,
        args_preview: JSON.stringify({
          command: "rm -rf build",
          workdir: "/tmp/project",
        }),
      },
    ];
    const { container } = render(<PythonApprovalBar chatId="chat" />);
    expect(screen.getByText(/^Script wants to run/).textContent).toContain(
      "shell",
    );
    const blocks = container.querySelectorAll("pre");
    expect(blocks[0]).toHaveTextContent("rm -rf build");
    expect(screen.getByText("workdir")).toBeInTheDocument();
    expect(screen.queryByText("command")).not.toBeInTheDocument();
    expect(container.textContent).not.toContain('{"command"');
  });
  it("shows file tool paths and a bounded content excerpt from valid JSON", () => {
    const content = Array.from({ length: 20 }, (_, i) => `line ${i}`).join(
      "\n",
    );
    approvals.list = [
      {
        ...pythonApproval,
        tool_name: "create_textdoc",
        command: "create_textdoc",
        editing_tool: true,
        rule: "default",
        args_preview: JSON.stringify({ content, path: "/tmp/a/__init__.py" }),
      },
    ];
    const { container } = render(<PythonApprovalBar chatId="chat" />);
    const keys = [...container.querySelectorAll("dt")].map(
      (node) => node.textContent,
    );
    expect(keys).toEqual(["path", "content"]);
    expect(screen.getByText("/tmp/a/__init__.py")).toBeInTheDocument();
    expect(screen.queryByText("line 12")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Show 12 more lines" }));
    expect(container.textContent).toContain("line 19");
    expect(screen.getByText(RUN_PERMIT_HINT)).toBeInTheDocument();
    expect(screen.queryByText(/Rule:/)).not.toBeInTheDocument();
    expect(container.textContent).not.toMatch(
      /create_textdoc\s*create_textdoc/,
    );
  });
  it("shows cut or invalid JSON as bounded preformatted text", () => {
    approvals.list = [
      {
        ...pythonApproval,
        tool_name: "update_textdoc",
        args_preview: '{"path":"/x","old_str":"abc',
      },
    ];
    const { container } = render(<PythonApprovalBar chatId="chat" />);
    expect(container.querySelector("pre")).toHaveTextContent(
      '{"path":"/x","old_str":"abc',
    );
  });
  it("shows a non-default rule", () => {
    render(<PythonApprovalBar chatId="chat" />);
    expect(screen.getByText("Rule: confirm")).toBeInTheDocument();
  });
  it("removes an unknown request locally and explains that it is no longer pending", async () => {
    vi.mocked(sendPythonApprovalDecision).mockRejectedValueOnce(
      new ChatCommandHttpError(400, "unknown request"),
    );
    render(<PythonApprovalBar chatId="chat" />);
    fireEvent.click(screen.getByRole("button", { name: "Allow once" }));
    await waitFor(() =>
      expect(screen.getByText(/no longer pending/i)).toBeInTheDocument(),
    );
    expect(
      screen.queryByRole("button", { name: "Allow once" }),
    ).not.toBeInTheDocument();
  });
  it("offers chat approval for editing tools and removes resolved requests", () => {
    approvals.editing = true;
    const view = render(<PythonApprovalBar chatId="chat" />);
    expect(
      screen.getByRole("button", { name: "Allow for this chat" }),
    ).toBeInTheDocument();
    approvals.pending = false;
    view.rerender(<PythonApprovalBar chatId="chat" />);
    expect(
      screen.queryByRole("button", { name: "Allow once" }),
    ).not.toBeInTheDocument();
  });
  it("restricts run and chat approval to editing tools", () => {
    render(<PythonApprovalBar chatId="chat" />);
    expect(
      screen.queryByRole("button", { name: "Allow for this chat" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Allow for this run" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText(RUN_PERMIT_HINT)).not.toBeInTheDocument();
  });
  it("offers Allow for this run for editing tools, described by the permit hint", async () => {
    approvals.editing = true;
    render(<PythonApprovalBar chatId="chat" />);
    const run = screen.getByRole("button", { name: "Allow for this run" });
    expect(run).toHaveAccessibleDescription(RUN_PERMIT_HINT);
    expect(
      screen.getByRole("button", { name: "Allow for this chat" }),
    ).toHaveAccessibleDescription(RUN_PERMIT_HINT);
    expect(RUN_PERMIT_HINT).toMatch(
      /every file edit this script makes, until it ends/,
    );
    const allow = screen.getByRole("group", { name: "Allow" });
    const refuse = screen.getByRole("group", { name: "Refuse" });
    expect(
      within(allow)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Allow once", "Allow for this run", "Allow for this chat"]);
    expect(
      within(refuse)
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Deny", "Stop script"]);
    expect(document.body.textContent).not.toMatch(/python/i);
    fireEvent.click(run);
    await waitFor(() =>
      expect(sendPythonApprovalDecision).toHaveBeenCalledWith(
        "chat",
        "q",
        "allow_for_run",
        expect.anything(),
        undefined,
      ),
    );
  });
  it("offers only Allow once, Deny and Stop script for a protected file, with its hint", () => {
    approvals.list = [
      { ...pythonApproval, editing_tool: true, needs_person: true },
    ];
    render(<PythonApprovalBar chatId="chat" />);
    expect(
      within(screen.getByRole("group", { name: "Allow" }))
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Allow once"]);
    expect(
      within(screen.getByRole("group", { name: "Refuse" }))
        .getAllByRole("button")
        .map((button) => button.textContent),
    ).toEqual(["Deny", "Stop script"]);
    expect(screen.getByText(PROTECTED_HINT)).toBeInTheDocument();
    expect(screen.queryByText(RUN_PERMIT_HINT)).toBeNull();
    expect(document.body.textContent).not.toMatch(/python/i);
  });
  it("keeps the permits and no protected hint when needs_person is false", () => {
    approvals.list = [
      { ...pythonApproval, editing_tool: true, needs_person: false },
    ];
    render(<PythonApprovalBar chatId="chat" />);
    expect(
      screen.getByRole("button", { name: "Allow for this run" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Allow for this chat" }),
    ).toBeInTheDocument();
    expect(screen.getByText(RUN_PERMIT_HINT)).toBeInTheDocument();
    expect(screen.queryByText(PROTECTED_HINT)).toBeNull();
  });
  it("reads needs_person tolerantly: true kept, anything invalid reads as false", () => {
    const base = { ...pythonApproval, editing_tool: true };
    expect(
      normalizePythonApprovalRequest({ ...base, needs_person: true })
        ?.needs_person,
    ).toBe(true);
    for (const value of ["yes", 1, null, false, undefined]) {
      const request = normalizePythonApprovalRequest({
        ...base,
        needs_person: value,
      });
      expect(request).toBeDefined();
      expect(request?.needs_person).toBeUndefined();
    }
  });
  it("shows the engine's reason for a refused decision without inviting a retry", async () => {
    approvals.editing = true;
    vi.mocked(sendPythonApprovalDecision).mockRejectedValueOnce(
      new ChatCommandHttpError(
        400,
        "Failed to send command: 400 Bad Request - ...",
        JSON.stringify({
          error: `AllowForRun cannot cover a change that needs a person's approval ${"x".repeat(
            600,
          )}`,
        }),
      ),
    );
    render(<PythonApprovalBar chatId="chat" />);
    fireEvent.click(screen.getByRole("button", { name: "Allow for this run" }));
    const alert = await screen.findByRole("alert");
    expect(alert).toHaveTextContent(
      "AllowForRun cannot cover a change that needs a person's approval",
    );
    expect(alert.textContent).not.toMatch(/try again/i);
    expect(alert.textContent?.length).toBeLessThan(400);
    expect(alert.querySelector("*")).toBeNull();
    expect(screen.getByRole("button", { name: "Deny" })).toBeEnabled();
  });
  it.each([
    ["a server error", new ChatCommandHttpError(500, "boom", '{"error":"x"}')],
    ["a network failure", new TypeError("Failed to fetch")],
    ["a 4xx without a message", new ChatCommandHttpError(403, "no", "")],
  ])("keeps the generic retry line for %s", async (_name, failure) => {
    vi.mocked(sendPythonApprovalDecision).mockRejectedValueOnce(failure);
    render(<PythonApprovalBar chatId="chat" />);
    fireEvent.click(screen.getByRole("button", { name: "Allow once" }));
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Unable to send this approval decision. Please try again.",
    );
  });
  it.each([
    ["Allow once", "allow_once"],
    ["Deny", "deny"],
    ["Stop script", "stop"],
  ])("sends %s", async (label, decision) => {
    render(<PythonApprovalBar chatId="chat" />);
    fireEvent.click(screen.getByRole("button", { name: label }));
    await waitFor(() =>
      expect(sendPythonApprovalDecision).toHaveBeenCalledWith(
        "chat",
        "q",
        decision,
        expect.anything(),
        undefined,
      ),
    );
  });
  it("shows a long command in full in a scrollable block, never clamped", () => {
    const command = Array.from({ length: 30 }, (_, i) => `echo step ${i}`).join(
      " &&\n",
    );
    approvals.list = [{ ...pythonApproval, command, args_preview: "{}" }];
    render(<PythonApprovalBar chatId="chat" />);
    const block = screen.getByLabelText("Command");
    expect(block.textContent).toBe(command);
    expect(screen.queryByRole("button", { name: /more lines/ })).toBeNull();
    expect(
      screen.getByText("30 lines · scroll to see all"),
    ).toBeInTheDocument();
  });
  it("says No arguments when there is nothing to show", () => {
    approvals.list = [
      {
        ...pythonApproval,
        tool_name: "tree",
        command: null,
        args_preview: "{}",
      },
    ];
    render(<PythonApprovalBar chatId="chat" />);
    expect(screen.getByText("No arguments")).toBeInTheDocument();
  });
  it("shows an uncut edit pair as a diff and falls back to blocks when cut", () => {
    approvals.list = [
      {
        ...pythonApproval,
        tool_name: "update_textdoc",
        editing_tool: true,
        args_preview: JSON.stringify({
          path: "/p/a.py",
          old_str: "a = 1\nb = 2\n",
          replacement: "a = 1\nb = 3\n",
        }),
      },
    ];
    const view = render(<PythonApprovalBar chatId="chat" />);
    expect(screen.getByLabelText("Proposed change")).toBeInTheDocument();
    expect(screen.queryByText("old_str")).toBeNull();
    approvals.list = [
      {
        ...pythonApproval,
        request_id: "q-cut",
        tool_name: "update_textdoc",
        editing_tool: true,
        args_preview: JSON.stringify({
          path: "/p/a.py",
          old_str: "a = 1… (+300 chars)",
          replacement: "a = 2",
        }),
      },
    ];
    view.rerender(<PythonApprovalBar chatId="chat" />);
    expect(screen.queryByLabelText("Proposed change")).toBeNull();
    expect(screen.getByText("old_str")).toBeInTheDocument();
    expect(screen.getByText("replacement")).toBeInTheDocument();
  });
  it("guards the buttons and moves focus when another request takes the place", () => {
    vi.useFakeTimers();
    try {
      approvals.list = [
        { ...pythonApproval, request_id: "a", tool_name: "cat" },
      ];
      const view = render(<PythonApprovalBar chatId="chat" />);
      const first = screen.getByRole("button", { name: "Allow once" });
      expect(first).toBeEnabled();
      fireEvent.pointerEnter(first);
      approvals.list = [
        { ...pythonApproval, request_id: "b", tool_name: "shell" },
      ];
      view.rerender(<PythonApprovalBar chatId="chat" />);
      const allow = screen.getByRole("button", { name: "Allow once" });
      expect(allow).toBeDisabled();
      fireEvent.click(allow);
      expect(sendPythonApprovalDecision).not.toHaveBeenCalled();
      expect(document.activeElement?.textContent).toContain(
        "Script wants to run",
      );
      expect(document.activeElement?.textContent).toContain("shell");
      act(() => {
        vi.advanceTimersByTime(APPROVAL_SWAP_GUARD_MS);
      });
      expect(screen.getByRole("button", { name: "Allow once" })).toBeEnabled();
    } finally {
      vi.useRealTimers();
    }
  });
  it("announces only a short text in the live region", () => {
    const { container } = render(<PythonApprovalBar chatId="chat" />);
    const live = container.querySelectorAll('[aria-live="polite"]');
    expect(live).toHaveLength(1);
    expect(live[0].textContent).toBe("Approval needed: shell");
  });
  it("clears the no-longer-pending notice when the next request shows", async () => {
    vi.mocked(sendPythonApprovalDecision).mockRejectedValueOnce(
      new ChatCommandHttpError(400, "unknown request"),
    );
    approvals.list = [{ ...pythonApproval, request_id: "a" }];
    const view = render(<PythonApprovalBar chatId="chat" />);
    fireEvent.click(screen.getByRole("button", { name: "Allow once" }));
    await waitFor(() =>
      expect(screen.getByText(/no longer pending/i)).toBeInTheDocument(),
    );
    approvals.list = [{ ...pythonApproval, request_id: "b" }];
    view.rerender(<PythonApprovalBar chatId="chat" />);
    expect(screen.queryByText(/no longer pending/i)).toBeNull();
  });
  describe("engine requests", () => {
    const region = () =>
      screen.getByRole("region", {
        name: "Script approval for engine.settings.set",
      });
    it("titles a one-item change, shows the function, scope and the value inline", () => {
      approvals.list = [engineRequest({})];
      render(<PythonApprovalBar chatId="chat" />);
      expect(
        screen.getByRole("heading", {
          name: /Script wants to change engine settings/,
        }),
      ).toBeInTheDocument();
      expect(region()).toHaveTextContent("engine.settings.set");
      expect(screen.getByText("Scope: project")).toBeInTheDocument();
      const item = screen.getByTestId("engine-item");
      expect(item).toHaveTextContent("autonomy.max_steps:25→changes to40");
      expect(
        screen.getByRole("button", { name: "Allow once" }).className,
      ).toMatch(/variant-primary/);
      expect(document.body.textContent).not.toMatch(/python/i);
    });
    it("never offers run or chat permits, even for an editing tool", () => {
      approvals.list = [engineRequest({}, { editing_tool: true })];
      render(<PythonApprovalBar chatId="chat" />);
      expect(
        screen.queryByRole("button", { name: /Allow for this/ }),
      ).toBeNull();
      expect(screen.queryByText(RUN_PERMIT_HINT)).toBeNull();
      expect(
        screen.getAllByRole("button").map((button) => button.textContent),
      ).toEqual(["Allow once", "Deny", "Stop script"]);
    });
    it("renders several items with not set, removed, JSON blocks and truncation", () => {
      approvals.list = [
        engineRequest({
          title: "change model defaults",
          items: [
            { label: "chat.model", new: "gpt-5" },
            { label: "chat.temperature", old: 0.2 },
            {
              label: "providers",
              old: {
                openai: {
                  enabled: true,
                  keys: ["a", "b", "c"],
                  base: "https://api.example.com/v1",
                },
              },
              new: { openai: { enabled: false } },
              truncated: true,
            },
          ],
        }),
      ];
      render(<PythonApprovalBar chatId="chat" />);
      const items = screen.getAllByTestId("engine-item");
      expect(items).toHaveLength(3);
      expect(within(items[0]).getByText("not set")).toBeInTheDocument();
      expect(within(items[0]).getByText('"gpt-5"')).toBeInTheDocument();
      expect(within(items[1]).getByText("removed")).toBeInTheDocument();
      expect(within(items[2]).getByText("Was")).toBeInTheDocument();
      expect(within(items[2]).getByText("Now")).toBeInTheDocument();
      expect(
        within(items[2]).getByText("Value cut by the engine"),
      ).toBeInTheDocument();
    });
    it("shows a multi-line string change as a diff", () => {
      approvals.list = [
        engineRequest({
          title: "change the system prompt",
          items: [
            {
              label: "prompts.agent",
              old: "You are helpful.\nBe brief.",
              new: "You are helpful.\nBe thorough.",
            },
          ],
        }),
      ];
      render(<PythonApprovalBar chatId="chat" />);
      expect(
        screen.getByLabelText("Changes to prompts.agent"),
      ).toBeInTheDocument();
      expect(
        screen.getByText("Be thorough.", { exact: false }),
      ).toBeInTheDocument();
    });
    it.each([
      [
        "risky",
        "Turning this off lets scripts run shell commands unchecked.",
        "Risky change",
      ],
      [
        "destructive",
        "Deletes the chat and its trajectory.",
        "Destructive change",
      ],
    ] as const)(
      "warns for a %s change, demotes Allow once and focuses Deny",
      async (engineClass, warning, label) => {
        approvals.list = [
          engineRequest({
            class: engineClass,
            title: "delete a chat",
            warning,
          }),
        ];
        render(<PythonApprovalBar chatId="chat" />);
        const deny = screen.getByRole("button", { name: "Deny" });
        await waitFor(() => expect(deny).toHaveFocus());
        expect(deny.className).toMatch(/variant-primary/);
        expect(
          screen.getByRole("button", { name: "Allow once" }).className,
        ).not.toMatch(/variant-primary/);
        const callout = region().querySelector("[data-tone]");
        expect(callout).toHaveTextContent(label);
        expect(callout).toHaveTextContent(warning);
        if (engineClass === "destructive")
          expect(callout).toHaveTextContent("This cannot be undone.");
        expect(document.querySelector("[aria-live]")).toHaveTextContent(
          `${label}: ${warning}`,
        );
      },
    );
    it("does not repeat the undo sentence when the warning says so", () => {
      approvals.list = [
        engineRequest({
          class: "destructive",
          title: "delete a chat",
          warning: "Deleting the chat cannot be undone.",
        }),
      ];
      render(<PythonApprovalBar chatId="chat" />);
      expect(region().textContent?.match(/undone/g)).toHaveLength(1);
    });
    it("never shows an engine tool as an ordinary request or offers permits for it", () => {
      const request = normalizePythonApprovalRequest({
        ...pythonApproval,
        tool_name: "engine.chats.delete",
        editing_tool: true,
        engine: { title: 42 },
      });
      if (!request) throw new Error("request was rejected");
      approvals.list = [request];
      render(<PythonApprovalBar chatId="chat" />);
      expect(
        screen.getByRole("heading", {
          name: /Script wants to change the engine/,
        }),
      ).toBeInTheDocument();
      expect(screen.queryByText(/Script wants to run/)).toBeNull();
      expect(screen.getByRole("alert")).toHaveTextContent(
        UNREADABLE_ENGINE_WARNING,
      );
      expect(
        screen.getAllByRole("button").map((button) => button.textContent),
      ).toEqual(["Allow once", "Deny", "Stop script"]);
    });
    it("withholds permits for an engine tool even without an engine part", () => {
      approvals.list = [
        {
          ...pythonApproval,
          tool_name: "engine.settings.set",
          editing_tool: true,
        },
      ];
      render(<PythonApprovalBar chatId="chat" />);
      expect(
        screen.queryByRole("button", { name: /Allow for this/ }),
      ).toBeNull();
      expect(screen.queryByText(RUN_PERMIT_HINT)).toBeNull();
    });
    it("reads an item without values as no value given and an unnamed one as unnamed", () => {
      approvals.list = [
        engineRequest({
          items: [{ label: "mystery" }, { label: "(unnamed)", new: 3 }],
        }),
      ];
      render(<PythonApprovalBar chatId="chat" />);
      const items = screen.getAllByTestId("engine-item");
      expect(items[0]).toHaveTextContent("mysteryno value given");
      expect(items[0]).not.toHaveTextContent("removed");
      expect(items[1]).toHaveTextContent("(unnamed)");
    });
    it("shows no protected-file hint for an engine change that needs a person", () => {
      approvals.list = [engineRequest({}, { needs_person: true })];
      render(<PythonApprovalBar chatId="chat" />);
      expect(screen.queryByText(PROTECTED_HINT)).toBeNull();
      expect(document.body.textContent).not.toMatch(/protected file/i);
    });
    it("shows a chat scope whole, with the id, and keeps the full text available", () => {
      const scope = `chat ${"t".repeat(
        80,
      )} (0f6c1a2e-3b4d-4e5f-8a9b-0c1d2e3f4a5b)`;
      approvals.list = [engineRequest({ scope })];
      render(<PythonApprovalBar chatId="chat" />);
      const chip = screen.getByText(/^Scope: chat/);
      expect(chip).toHaveTextContent(`Scope: ${scope}`);
      expect(chip).toHaveAttribute("title", scope);
      expect(chip).toHaveAttribute("aria-description", `Scope: ${scope}`);
    });
    it("says how many entries could not be read", () => {
      approvals.list = [engineRequest({ items: [], items_dropped: 1 })];
      render(<PythonApprovalBar chatId="chat" />);
      expect(
        screen.getByText(/1 more entry could not be read and is not shown/),
      ).toBeInTheDocument();
    });
    it("cuts a 5,000-character label, title and function, keeping the actions reachable", () => {
      const label = "k".repeat(5000);
      approvals.list = [
        engineRequest(
          {
            title: `change ${"t".repeat(5000)}`,
            function: `settings.${"f".repeat(5000)}`,
            scope: "s".repeat(5000),
            items: [{ label, old: 1, new: 2 }],
          },
          { tool_name: `engine.settings.${"f".repeat(5000)}` },
        ),
      ];
      const { container } = render(<PythonApprovalBar chatId="chat" />);
      const item = screen.getByTestId("engine-item");
      expect(item.textContent?.length).toBeLessThan(300);
      expect(item).toHaveTextContent("Show all 5000 characters");
      fireEvent.click(within(item).getByRole("button", { name: /Show all/ }));
      expect(item.textContent).toContain(label);
      const heading = screen.getByRole("heading");
      expect(heading.textContent?.length).toBeLessThan(400);
      expect(container.textContent).not.toContain("s".repeat(200));
      expect(screen.getByRole("button", { name: "Deny" })).toBeInTheDocument();
      expect(container.querySelector("[class*=content]")).toContainElement(
        item,
      );
      expect(container.querySelector("[class*=content]")).not.toContainElement(
        screen.getByRole("button", { name: "Deny" }),
      );
    });
    it("keeps a chats.patch request with long multi-line texts readable and its actions outside the scroll area", () => {
      approvals.list = [chatsPatchRequest];
      const { container } = render(<PythonApprovalBar chatId="chat" />);
      const prompt = screen.getByRole("region", {
        name: "Script approval for engine.chats.patch",
      });
      expect(prompt.querySelector("[data-tone=danger]")).toHaveTextContent(
        "Destructive change. The original messages will be overwritten and cannot be restored.",
      );
      const [edit, removed, inserted] = screen.getAllByTestId("engine-item");
      const diff = within(edit).getByLabelText("Changes to message 2 (user)");
      expect(diff).toHaveTextContent("Keep the public API unchanged.");
      expect(diff).toHaveTextContent(
        "Keep the public API unchanged; deprecate nothing.",
      );
      expect(within(diff).getByText(PATCH_LONG_LINE).className).toMatch(
        /lineContent/,
      );
      expect(diff).toHaveTextContent("@@ -2 +2 @@+1−1");
      expect(diff).toHaveTextContent("@@ -5 +5 @@+1−1");
      expect(within(removed).getByText("removed")).toBeInTheDocument();
      expect(within(removed).getByText("Was")).toBeInTheDocument();
      expect(
        within(removed).getByRole("button", { name: "Show 6 more lines" }),
      ).toBeInTheDocument();
      expect(within(inserted).getByText("not set")).toBeInTheDocument();
      expect(inserted).toHaveTextContent("Do not touch the CLI.");
      const content = container.querySelector("[class*=content]");
      for (const item of [edit, removed, inserted])
        expect(content).toContainElement(item);
      for (const name of ["Allow once", "Deny", "Stop script"])
        expect(content).not.toContainElement(
          screen.getByRole("button", { name }),
        );
      expect(
        screen.queryByRole("button", { name: /Allow for this/ }),
      ).toBeNull();
    });
    it("shows the first 20 changes of a text pair and the rest on request", () => {
      const before = Array.from({ length: 60 }, (_, index) => `line ${index}`);
      const after = before.map((line, index) =>
        index % 2 === 0 ? `${line} changed` : line,
      );
      approvals.list = [
        engineRequest({
          title: "change a long message",
          items: [
            { label: "message", old: before.join("\n"), new: after.join("\n") },
          ],
        }),
      ];
      render(<PythonApprovalBar chatId="chat" />);
      const diff = screen.getByLabelText("Changes to message");
      expect(diff.querySelectorAll("[class*=diffBlock]")).toHaveLength(20);
      fireEvent.click(
        within(diff).getByRole("button", { name: "Show 10 more changes" }),
      );
      expect(diff.querySelectorAll("[class*=diffBlock]")).toHaveLength(30);
    });
    it("shows bidi overrides and zero-width characters in labels, titles and values as markers", () => {
      approvals.list = [
        engineRequest({
          title: "delete \u202Eexe.terces\u202C chat",
          warning: "Gone\u200B for good.",
          class: "destructive",
          items: [{ label: "na\u202Eme", old: "a\u200Bb", new: "ab" }],
        }),
      ];
      const { container } = render(<PythonApprovalBar chatId="chat" />);
      const text = container.textContent ?? "";
      expect(text).not.toMatch(/[\u202A-\u202E\u200B]/);
      expect(text).toContain("delete ⟨U+202E⟩exe.terces⟨U+202C⟩ chat");
      expect(text).toContain("na⟨U+202E⟩me");
      expect(text).toContain('"a⟨U+200B⟩b"');
      expect(text).toContain("Gone⟨U+200B⟩ for good.");
    });
    it.each(["risky", "destructive"] as const)(
      "ties the %s warning to the focused Deny button",
      async (engineClass) => {
        approvals.list = [
          engineRequest({ class: engineClass, warning: "Danger ahead." }),
        ];
        render(<PythonApprovalBar chatId="chat" />);
        const deny = screen.getByRole("button", { name: "Deny" });
        await waitFor(() => expect(deny).toHaveFocus());
        const callout = screen.getByRole("alert");
        expect(callout).toHaveTextContent("Danger ahead.");
        expect(deny).toHaveAccessibleDescription(
          expect.stringContaining("Danger ahead."),
        );
        expect(deny).toHaveAccessibleDescription(
          expect.stringContaining("Script wants to change engine settings"),
        );
        expect(
          screen.getByRole("button", { name: "Allow once" }),
        ).toHaveAccessibleDescription(expect.stringContaining("Danger ahead."));
      },
    );
    it("never takes focus from a field the user is typing in", async () => {
      const input = document.createElement("textarea");
      document.body.appendChild(input);
      try {
        input.focus();
        approvals.list = [
          engineRequest({ class: "destructive", warning: "Deletes it." }),
        ];
        render(<PythonApprovalBar chatId="chat" />);
        await asyncAct(async () => {
          await new Promise((done) => setTimeout(done, 0));
        });
        expect(input).toHaveFocus();
        expect(screen.getByRole("alert")).toHaveTextContent("Deletes it.");
      } finally {
        input.remove();
      }
    });
    it("never moves focus under a pointer press in progress", async () => {
      approvals.list = [];
      const view = render(<PythonApprovalBar chatId="chat" />);
      fireEvent.pointerDown(document.body);
      try {
        approvals.list = [engineRequest({ class: "risky", warning: "x" })];
        view.rerender(<PythonApprovalBar chatId="chat" />);
        await asyncAct(async () => {
          await new Promise((done) => setTimeout(done, 0));
        });
        expect(screen.getByRole("button", { name: "Deny" })).not.toHaveFocus();
      } finally {
        fireEvent.pointerUp(document.body);
      }
    });
    it("leaves focus alone for an ordinary engine change", () => {
      approvals.list = [engineRequest({})];
      render(<PythonApprovalBar chatId="chat" />);
      expect(screen.getByRole("button", { name: "Deny" })).not.toHaveFocus();
      expect(screen.queryByRole("alert")).toBeNull();
    });
    it("sends deny for an engine change", async () => {
      approvals.list = [engineRequest({ class: "risky", warning: "x" })];
      render(<PythonApprovalBar chatId="chat" />);
      fireEvent.click(screen.getByRole("button", { name: "Deny" }));
      await waitFor(() =>
        expect(sendPythonApprovalDecision).toHaveBeenCalledWith(
          "chat",
          "e1",
          "deny",
          expect.anything(),
          undefined,
        ),
      );
    });
  });
});
