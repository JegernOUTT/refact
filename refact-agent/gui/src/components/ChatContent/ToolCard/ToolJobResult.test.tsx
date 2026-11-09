import { afterEach, describe, expect, it } from "vitest";
import { cleanup } from "@testing-library/react";
import { render, screen } from "../../../utils/test-utils";
import type { NativeToolJobResult } from "../ChatContentDisplayItems";
import { ToolJobResult } from "./ToolJobResult";

afterEach(cleanup);

describe("native tool job result", () => {
  it("uses a compact tool card with Markdown output once and no human retry/branch/delete actions", () => {
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
    expect(
      screen.getAllByRole("heading", { name: "Native completed output" }),
    ).toHaveLength(1);
    expect(
      screen.queryByRole("button", { name: /retry|branch|delete|cancel/i }),
    ).toBeNull();
    expect(
      screen.getByTestId("tool-job-result").closest('[data-density="compact"]'),
    ).toBeTruthy();
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
    expect(screen.getAllByTestId("context-file")).toHaveLength(1);
    expect(screen.getByTitle("src/native.ts")).toBeTruthy();
    expect(await screen.findByText(/native file preview/)).toBeTruthy();
  });
});
