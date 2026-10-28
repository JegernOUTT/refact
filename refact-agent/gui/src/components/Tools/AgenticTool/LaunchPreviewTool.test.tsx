import { describe, expect, test } from "vitest";
import { fireEvent, within } from "@testing-library/react";
import {
  createDefaultChatState,
  render,
  screen,
} from "../../../utils/test-utils";
import { LaunchPreviewTool } from "./LaunchPreviewTool";
import { launchPreviewCall, launchPreviewReceipt } from "./launchFixtures";

const resolved = { ...launchPreviewReceipt().extra, skipped_count: 0 };

function renderOpen(extra: unknown, rawContent?: string) {
  const view = render(
    <LaunchPreviewTool
      toolCall={launchPreviewCall}
      rawContent={rawContent}
      extra={extra}
      hasResult
      failed={false}
    />,
  );
  const toggle = screen.getByTestId("transcript-toggle");
  if (toggle.getAttribute("aria-expanded") === "false") fireEvent.click(toggle);
  return view;
}

describe("LaunchPreviewTool", () => {
  test("row reads Launch · task · preview and stays quiet when resolved", () => {
    render(
      <LaunchPreviewTool
        toolCall={launchPreviewCall}
        extra={resolved}
        hasResult
        failed={false}
      />,
    );
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Launch");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Inspect accepted context",
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent("preview");
    expect(screen.queryByTestId("transcript-status-pill")).toBeNull();
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  test("unresolved context opens the item with a Blocked pill", () => {
    render(
      <LaunchPreviewTool
        toolCall={launchPreviewCall}
        extra={launchPreviewReceipt().extra}
        hasResult
        failed={false}
      />,
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Blocked",
    );
    expect(screen.getByTestId("launch-blocked")).toHaveTextContent(
      "1 unresolved context entry",
    );
  });

  test("pages tool authority and context sources independently", () => {
    renderOpen(resolved);
    expect(screen.getByText("cat")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Next tool" }));
    expect(screen.getByText("mcp_fixture_read")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next tool" })).toBeDisabled();
    expect(screen.getByText("src/accepted-context.rs")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Next source" }));
    expect(screen.getByText("completed-parent-message")).toBeInTheDocument();
    expect(screen.queryByText("src/accepted-context.rs")).toBeNull();
  });

  test.each([
    [undefined, "Tool authority not recorded"],
    [[], "No tools permitted"],
  ])("distinguishes absent authority from deny-all", (tools, text) => {
    renderOpen({ ...resolved, effective_tools: tools });
    expect(screen.getByLabelText("Preview tool authority")).toHaveTextContent(
      text,
    );
  });

  test("renders the declared deliverable contract", () => {
    renderOpen(resolved);
    const contract = screen.getByLabelText("Preview deliverable contract");
    expect(contract).toHaveTextContent("Findings, Evidence");
    expect(contract).toHaveTextContent("declared");
  });

  test("renders effective launch settings", () => {
    renderOpen({
      ...resolved,
      effective_launch: {
        schema_version: 1,
        revision: 1,
        model: "local/qualified-model",
        mode: "agent",
        tools: { Only: ["cat", "fixture_read"] },
        max_steps: 256,
        subchat_depth: 1,
        stateful: true,
        autonomous_no_confirm: false,
        auto_approve_editing_tools: true,
        auto_approve_dangerous_commands: false,
        worktree_mode: "isolated(auto_merge=true)",
        parent_worktree_id: null,
        project_root: "/project",
        messages: 4,
        accepted_bytes: 3360,
        accepted_byte_limit: 16777216,
      },
    });
    const settings = screen.getByLabelText("Effective launch settings");
    expect(settings).toHaveTextContent("1 (schema 1)");
    expect(settings).toHaveTextContent("2 concrete");
    expect(settings).toHaveTextContent(
      "edits auto-approved, commands confirmed",
    );
    expect(settings).toHaveTextContent("3360 of 16777216 bytes, 4 messages");
  });

  test.each([
    null,
    {},
    { ...resolved, preview: false },
    { ...resolved, estimated_tokens: -1 },
    { ...resolved, effective_tools: [42] },
    { ...resolved, effective_launch: { revision: "one" } },
    {
      ...resolved,
      context_sources: [{ type: "arbitrary", observed_at_ms: 0 }],
    },
  ])("rejects malformed metadata with one raw copy", (extra) => {
    const { container } = renderOpen(extra, "Original diagnostic ERROR_TAIL");
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Failed",
    );
    expect(screen.getByRole("alert")).toHaveTextContent(
      "does not confirm a launch",
    );
    expect(screen.queryByLabelText("Effective launch settings")).toBeNull();
    const raw = screen.getByTestId("transcript-raw");
    fireEvent.click(within(raw).getByRole("button", { name: "Raw" }));
    expect(raw).toHaveTextContent("ERROR_TAIL");
    expect(
      container.querySelectorAll("[data-testid=transcript-raw]"),
    ).toHaveLength(1);
  });

  test.each([
    [false, "No result"],
    [true, "Running"],
  ] as const)(
    "missing result while busy=%s shows %s without claiming a preview",
    (busy, pill) => {
      const chat = createDefaultChatState();
      const runtime = chat.threads[chat.current_thread_id];
      runtime.streaming = busy;
      runtime.thread.messages = [
        { role: "assistant", content: "", tool_calls: [launchPreviewCall] },
      ];
      render(
        <LaunchPreviewTool
          toolCall={launchPreviewCall}
          extra={undefined}
          hasResult={false}
          failed={false}
        />,
        { preloadedState: { chat } },
      );
      expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
        pill,
      );
      expect(screen.queryByTestId("launch-blocked")).toBeNull();
    },
  );

  test("bounds task and source text and keeps markup inert", () => {
    const { container } = render(
      <LaunchPreviewTool
        toolCall={{
          ...launchPreviewCall,
          function: {
            ...launchPreviewCall.function,
            arguments: JSON.stringify({
              task: "<script>unsafe</script>" + "x".repeat(10000),
            }),
          },
        }}
        extra={{
          ...resolved,
          context_sources: [
            {
              type: "raw_text",
              label: "<script>label</script>" + "y".repeat(10000),
              observed_at_ms: 1,
            },
          ],
        }}
        hasResult
        failed={false}
      />,
    );
    expect(
      screen.getByTestId("transcript-target").textContent?.length,
    ).toBeLessThanOrEqual(160);
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    const label = within(
      screen.getByLabelText("Launch context source"),
    ).getByText(/<script>label/u);
    expect(label.textContent?.length).toBeLessThanOrEqual(241);
    expect(container.querySelector("script")).toBeNull();
  });
});
