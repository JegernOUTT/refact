import { describe, expect, test } from "vitest";
import { fireEvent } from "@testing-library/react";
import { render, screen } from "../../../utils/test-utils";
import { LaunchPreviewTool } from "./LaunchPreviewTool";
import { launchPreviewCall, launchPreviewReceipt } from "./launchFixtures";

describe("LaunchPreviewTool", () => {
  test("pages concrete authority independently of context sources", () => {
    render(
      <LaunchPreviewTool
        toolCall={launchPreviewCall}
        extra={launchPreviewReceipt().extra}
        hasResult
        failed={false}
      />,
    );
    expect(screen.getByText("cat")).toBeInTheDocument();
    expect(screen.queryByText("mcp_fixture_read")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Next tool" }));
    expect(screen.getByText("mcp_fixture_read")).toBeInTheDocument();
    expect(screen.getByText("src/accepted-context.rs")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Next tool" })).toBeDisabled();
  });

  test.each([
    [undefined, "Effective tool authority was not recorded"],
    [[], "No tools permitted"],
  ])("distinguishes absent authority from deny-all", (effectiveTools, text) => {
    render(
      <LaunchPreviewTool
        toolCall={launchPreviewCall}
        extra={{
          ...launchPreviewReceipt().extra,
          effective_tools: effectiveTools,
        }}
        hasResult
        failed={false}
      />,
    );
    expect(screen.getByLabelText("Preview tool authority")).toHaveTextContent(
      String(text),
    );
  });

  test("shows one source at a time without claiming acceptance", () => {
    render(
      <LaunchPreviewTool
        toolCall={launchPreviewCall}
        extra={launchPreviewReceipt().extra}
        hasResult
        failed={false}
      />,
    );
    expect(screen.getByLabelText("Child launch preview")).toHaveTextContent(
      "No child spawned",
    );
    expect(screen.getByText("src/accepted-context.rs")).toBeInTheDocument();
    expect(screen.queryByText("completed-parent-message")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Next source" }));
    expect(screen.getByText("completed-parent-message")).toBeInTheDocument();
    expect(screen.queryByText("src/accepted-context.rs")).toBeNull();
    expect(screen.getByRole("button", { name: "Next source" })).toBeDisabled();
    expect(
      screen.getByText(/Unresolved requested context prevents launch/u),
    ).toBeInTheDocument();
  });

  test.each([
    null,
    {},
    { ...launchPreviewReceipt().extra, preview: false },
    { ...launchPreviewReceipt().extra, estimated_tokens: -1 },
    { ...launchPreviewReceipt().extra, effective_tools: "all" },
    { ...launchPreviewReceipt().extra, effective_tools: [42] },
    { ...launchPreviewReceipt().extra, deliverable: "sections" },
    {
      ...launchPreviewReceipt().extra,
      deliverable: { required_sections: [1], evidence_required: true },
    },
    {
      ...launchPreviewReceipt().extra,
      context_sources: [{ type: "arbitrary", observed_at_ms: 0 }],
    },
  ])(
    "rejects malformed and legacy metadata without a success claim",
    (extra) => {
      render(
        <LaunchPreviewTool
          toolCall={launchPreviewCall}
          extra={extra}
          hasResult
          failed={false}
        />,
      );
      expect(screen.getByRole("alert")).toHaveTextContent(
        "does not confirm a launch",
      );
      expect(screen.queryByText("Context resolved")).toBeNull();
    },
  );

  test("renders the declared deliverable contract without a verdict", () => {
    render(
      <LaunchPreviewTool
        toolCall={launchPreviewCall}
        extra={launchPreviewReceipt().extra}
        hasResult
        failed={false}
      />,
    );
    const contract = screen.getByLabelText("Preview deliverable contract");
    expect(contract).toHaveTextContent("Findings, Evidence");
    expect(contract).toHaveTextContent("structural subset enforced");
    expect(contract).toHaveTextContent("unbacked claims stay unverified");
  });

  test("reports an undeclared contract without inventing one", () => {
    const rest = { ...launchPreviewReceipt().extra };
    delete rest.deliverable;
    render(
      <LaunchPreviewTool
        toolCall={launchPreviewCall}
        extra={rest}
        hasResult
        failed={false}
      />,
    );
    expect(
      screen.getByLabelText("Preview deliverable contract"),
    ).toHaveTextContent("No deliverable contract declared");
  });

  test("renders effective launch settings from the real preparation path", () => {
    const extra = {
      ...launchPreviewReceipt().extra,
      effective_launch: {
        schema_version: 1,
        revision: 1,
        model: "local/qualified-model",
        mode: "agent",
        tools: { Only: ["cat", "mcp_fixture_read"] },
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
    };
    render(
      <LaunchPreviewTool
        toolCall={launchPreviewCall}
        extra={extra}
        hasResult
        failed={false}
      />,
    );
    const settings = screen.getByLabelText("Effective launch settings");
    expect(settings).toHaveTextContent("1 (schema 1)");
    expect(settings).toHaveTextContent("2 concrete");
    expect(settings).toHaveTextContent("256");
    expect(settings).toHaveTextContent("isolated(auto_merge=true)");
    expect(settings).toHaveTextContent(
      "edits auto-approved, commands confirmed",
    );
    expect(settings).toHaveTextContent("3360 of 16777216 bytes, 4 messages");
    expect(screen.queryByTestId("launch-blocked")).toBeNull();
  });

  test("reports a blocked launch and rejects malformed effective settings", () => {
    const blocked = render(
      <LaunchPreviewTool
        toolCall={launchPreviewCall}
        extra={{
          ...launchPreviewReceipt().extra,
          launch_blocked: "subchat depth limit (5) exceeded",
        }}
        hasResult
        failed={false}
      />,
    );
    expect(screen.getByTestId("launch-blocked")).toHaveTextContent(
      "subchat depth limit (5) exceeded",
    );
    blocked.unmount();
    const { container } = render(
      <LaunchPreviewTool
        toolCall={launchPreviewCall}
        extra={{
          ...launchPreviewReceipt().extra,
          effective_launch: { revision: "one" },
        }}
        hasResult
        failed={false}
      />,
    );
    expect(container).toHaveTextContent(/unavailable|malformed|could not/i);
    expect(
      screen.queryByLabelText("Effective launch settings"),
    ).not.toBeInTheDocument();
  });

  test("keeps pending and failed distinct from a resolved preview", () => {
    const props = {
      toolCall: launchPreviewCall,
      extra: launchPreviewReceipt().extra,
    };
    const { rerender } = render(
      <LaunchPreviewTool {...props} hasResult={false} failed={false} />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      "No acceptance receipt yet",
    );
    rerender(<LaunchPreviewTool {...props} hasResult failed />);
    expect(screen.getByRole("alert")).toBeInTheDocument();
  });

  test("bounds source text and renders markup as text", () => {
    const extra = {
      ...launchPreviewReceipt().extra,
      context_sources: [
        {
          type: "raw_text",
          label: "<script>unsafe</script>" + "x".repeat(10000),
          observed_at_ms: 1,
        },
      ],
    };
    const { container } = render(
      <LaunchPreviewTool
        toolCall={launchPreviewCall}
        extra={extra}
        hasResult
        failed={false}
      />,
    );
    const label = screen.getByText(/<script>unsafe/u);
    expect(label.textContent?.length).toBeLessThanOrEqual(241);
    expect(container.querySelector("script")).toBeNull();
  });
});
