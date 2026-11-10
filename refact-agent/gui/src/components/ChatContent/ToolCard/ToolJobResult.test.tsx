import { afterEach, describe, expect, it } from "vitest";
import { cleanup } from "@testing-library/react";
import { render, screen, fireEvent } from "../../../utils/test-utils";
import type { ChatMessages, ToolCall } from "../../../services/refact/types";
import type { NativeToolJobResult } from "../ChatContentDisplayItems";
import { buildDisplayItems } from "../ChatContentDisplayItems";
import { ToolJobResult } from "./ToolJobResult";

afterEach(cleanup);

function appCompletion(
  name: string,
  args: Record<string, unknown>,
  content: unknown,
  failed = false,
): NativeToolJobResult {
  const call: ToolCall = {
    id: "app-call",
    index: 0,
    type: "function",
    function: { name, arguments: JSON.stringify(args) },
  };
  const messages = [
    { role: "assistant", content: "", tool_calls: [call] },
    {
      role: "event",
      message_id: "tool_job:app-job:2:0",
      content,
      tool_failed: failed,
      event: {
        subkind: "system_notice",
        source: "tool_job.completed",
        payload: { actor: "tool_completion", job_id: "app-job" },
      },
      tool_job_result: {
        actor: "tool_completion",
        job_id: "app-job",
        revision: 2,
        tool_name: name,
        tool_call_id: call.id,
        original_role: "tool",
      },
      delivery: { source: "tool_job.completed", id: "tool_job:app-job:2" },
    },
  ];
  const item = buildDisplayItems(messages as ChatMessages, false).find(
    (candidate) => candidate.type === "tool_job_result",
  );
  if (item?.type !== "tool_job_result") {
    throw new Error("App completion did not reach the native result renderer");
  }
  return item.result;
}

describe("native tool job result", () => {
  it("starts collapsed and toggles native Markdown output without human retry/branch/delete actions", () => {
    const result: NativeToolJobResult = {
      toolName: "code_health",
      message: {
        role: "tool",
        tool_call_id: "call",
        content: "## Native completed output",
      },
    };
    render(<ToolJobResult result={result} />);
    expect(screen.getAllByText("Tool result")).toHaveLength(1);
    expect(screen.getByText("code_health")).toBeTruthy();
    const toggle = screen.getByRole("button", {
      name: /Tool result.*code_health/,
    });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByTestId("tool-job-result")).toBeNull();
    expect(toggle.closest('[data-density="compact"]')).toBeTruthy();
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(
      screen.getAllByRole("heading", { name: "Native completed output" }),
    ).toHaveLength(1);
    expect(
      screen.queryByRole("button", { name: /retry|branch|delete|cancel/i }),
    ).toBeNull();
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    fireEvent.transitionEnd(screen.getByTestId("transcript-panel"));
    expect(screen.queryByTestId("tool-job-result")).toBeNull();
    fireEvent.click(toggle);
    expect(
      screen.getAllByRole("heading", { name: "Native completed output" }),
    ).toHaveLength(1);
  });

  it("keeps failed output collapsed while preserving the failure indicator", () => {
    render(
      <ToolJobResult
        result={{
          toolName: "process_read",
          message: {
            role: "tool",
            tool_call_id: "call",
            tool_failed: true,
            content: "## Failed process output",
          },
        }}
      />,
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Failed",
    );
    const toggle = screen.getByRole("button", {
      name: /Tool result.*process_read/,
    });
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByTestId("tool-job-result")).toBeNull();
    fireEvent.click(toggle);
    expect(
      screen.getByRole("heading", { name: "Failed process output" }),
    ).toBeInTheDocument();
  });

  it("preserves the user's toggle when the result updates", () => {
    const result: NativeToolJobResult = {
      toolName: "process_read",
      message: {
        role: "tool",
        tool_call_id: "call",
        content: "## Original output",
      },
    };
    const { rerender } = render(<ToolJobResult result={result} />);
    const updated: NativeToolJobResult = {
      ...result,
      message: {
        role: "tool",
        tool_call_id: "call",
        content: "## Updated output",
      },
    };
    rerender(<ToolJobResult result={updated} />);
    const toggle = screen.getByTestId("transcript-toggle");
    expect(toggle).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByTestId("tool-job-result")).toBeNull();
    fireEvent.click(toggle);
    rerender(<ToolJobResult result={result} />);
    expect(toggle).toHaveAttribute("aria-expanded", "true");
    expect(
      screen.getByRole("heading", { name: "Original output" }),
    ).toBeInTheDocument();
  });

  it("renders native multimodal text and image without JSON flattening", () => {
    render(
      <ToolJobResult
        result={{
          toolName: "chrome",
          message: {
            role: "tool",
            tool_call_id: "call",
            content: [
              { m_type: "text", m_content: "## Screenshot result" },
              { m_type: "image/png", m_content: "aGVsbG8=" },
            ],
          },
        }}
      />,
    );
    expect(screen.queryByTestId("tool-job-result")).toBeNull();
    expect(document.querySelector("img")).toBeNull();
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    expect(
      screen.getByRole("heading", { name: "Screenshot result" }),
    ).toBeTruthy();
    expect(document.querySelector("img")?.getAttribute("src")).toBe(
      "data:image/png;base64,aGVsbG8=",
    );
  });

  it("reuses native context files, including the original path and preview", async () => {
    render(
      <ToolJobResult
        result={{
          toolName: "cat",
          message: {
            role: "context_file",
            tool_call_id: "call",
            content: [
              {
                file_name: "src/native.ts",
                file_content: "native file preview",
                line1: 1,
                line2: 1,
              },
            ],
          },
        }}
      />,
    );
    expect(screen.queryByTestId("context-file")).toBeNull();
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    expect(screen.getAllByTestId("context-file")).toHaveLength(1);
    expect(screen.getByTitle("src/native.ts")).toBeTruthy();
    expect(await screen.findByText(/native file preview/)).toBeTruthy();
  });

  it("starts flattened App completions collapsed and expands structured cards with the original action", () => {
    const { container } = render(
      <ToolJobResult
        result={appCompletion(
          "app",
          { action: "read_file", path: "app.json" },
          JSON.stringify({
            revision: "app-revision",
            path: "app.json",
            content: '{"name":"Sample app","host_view_token":"PRIVATE"}',
          }),
        )}
      />,
    );
    expect(screen.getByTestId("app-tool")).toBeInTheDocument();
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.getByText("App · Read file")).toBeInTheDocument();
    expect(screen.queryByText("Resource")).toBeNull();
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "true",
    );
    expect(screen.getByText("Resource")).toBeInTheDocument();
    expect(screen.getByText("app.json")).toBeInTheDocument();
    expect(screen.getByText("app-revision")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Raw JSON" }));
    expect(container.textContent).not.toContain("PRIVATE");
    expect(
      screen.queryByRole("button", { name: /retry|branch|delete|cancel/i }),
    ).toBeNull();
  });

  it("renders a flattened App screenshot completion once with PNG and capture metadata", () => {
    render(
      <ToolJobResult
        result={appCompletion("app_screenshot", { action: "capture" }, [
          {
            m_type: "text",
            m_content: '{"revision":"capture-revision","status":"ok"}',
          },
          { m_type: "image/png", m_content: "iVBORw0KGgo=" },
        ])}
      />,
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "PNG captured",
    );
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.queryByText("capture-revision")).toBeNull();
    expect(document.querySelector("img")).toBeNull();
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    expect(screen.getByText("capture-revision")).toBeInTheDocument();
    expect(document.querySelectorAll("img")).toHaveLength(1);
    expect(document.querySelector("img")).toHaveAttribute(
      "src",
      "data:image/png;base64,iVBORw0KGgo=",
    );
    expect(screen.queryByTestId("event-row")).toBeNull();
  });

  it("preserves failed App completion status rather than showing success", () => {
    render(
      <ToolJobResult
        result={appCompletion(
          "app_state",
          { action: "get" },
          "Error: app not published",
          true,
        )}
      />,
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Tool failed",
    );
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(screen.queryByRole("button", { name: "Raw JSON" })).toBeNull();
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    fireEvent.click(screen.getByRole("button", { name: "Raw JSON" }));
    expect(screen.getByText(/Error: app not published/)).toBeInTheDocument();
  });

  it("keeps withheld App completion output behind the privacy reveal action", () => {
    const result = appCompletion(
      "app",
      { action: "read_file" },
      "Output withheld by user privacy policy",
    );
    result.message.extra = {
      ...result.message.extra,
      privacy_shell: { withheld: true, local_only_output: "LOCAL_PRIVATE" },
    };
    render(<ToolJobResult result={result} />);
    expect(
      screen.getByTestId("privacy-withheld-output-card"),
    ).toBeInTheDocument();
    expect(screen.queryByTestId("app-tool")).toBeNull();
    expect(screen.queryByText("LOCAL_PRIVATE")).toBeNull();
    expect(screen.queryByRole("button", { name: "Raw JSON" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show me" }));
    expect(screen.getByTestId("privacy-local-output")).toHaveTextContent(
      "LOCAL_PRIVATE",
    );
  });

  it("does not send a privacy refusal through the normal App response preview", () => {
    render(
      <ToolJobResult
        result={appCompletion(
          "app_screenshot",
          { action: "capture" },
          "Output withheld by user privacy policy: guarded screenshot",
        )}
      />,
    );
    expect(screen.queryByTestId("app-tool")).toBeNull();
    expect(screen.queryByRole("button", { name: "Raw JSON" })).toBeNull();
    expect(document.querySelector("img")).toBeNull();
  });
});
