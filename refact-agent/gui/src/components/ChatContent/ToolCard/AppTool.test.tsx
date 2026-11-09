import { describe, expect, it, vi } from "vitest";
import { render, screen, fireEvent } from "../../../utils/test-utils";
import type { ToolCall, ToolResult } from "../../../services/refact/types";
import { AppTool } from "./AppTool";

const call: ToolCall = {
  id: "app-test",
  index: 0,
  type: "function",
  function: {
    name: "app",
    arguments: '{"action":"read_file","password":"ARG_SECRET"}',
  },
};
const receipt = (content: ToolResult["content"], failed = false): ToolResult =>
  typeof content === "string"
    ? { tool_call_id: call.id ?? "", content, tool_failed: failed }
    : { tool_call_id: call.id ?? "", content, tool_failed: failed };
describe("compact App card", () => {
  it("starts collapsed, redacts resource JSON/HTML and copies only defensive response", () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    const { container } = render(
      <AppTool
        toolCall={call}
        result={receipt(
          JSON.stringify({
            revision: "r",
            path: "ui/index.html",
            content:
              '<script>const config={"host_view_token":"DO_NOT_SHOW"}</script>',
            view_token: "HOST_SECRET",
          }),
        )}
      />,
    );
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(container.querySelector('[data-density="compact"]')).not.toBeNull();
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    fireEvent.click(screen.getByRole("button", { name: "Raw JSON" }));
    expect(container.textContent).not.toMatch(
      /DO_NOT_SHOW|ARG_SECRET|HOST_SECRET/,
    );
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: { writeText },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /Copy Permitted response/ }),
    );
    expect(writeText).toHaveBeenCalledOnce();
    expect(String(writeText.mock.calls[0][0])).not.toMatch(
      /DO_NOT_SHOW|ARG_SECRET|HOST_SECRET/,
    );
    expect(container.querySelector("button button")).toBeNull();
  });
  it("renders actual PNG inside the App card with app-reported revision metadata", () => {
    render(
      <AppTool
        toolCall={{
          ...call,
          function: {
            name: "app_screenshot",
            arguments: '{"action":"capture"}',
          },
        }}
        result={receipt([
          {
            m_type: "text",
            m_content:
              '{"revision":"published-r","trust":"app_reported","status":"ok"}',
          },
          { m_type: "image/png", m_content: "iVBORw0KGgo=" },
        ])}
      />,
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "PNG captured",
    );
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    expect(
      screen.getByTestId("app-tool-images").querySelector("img"),
    ).toHaveAttribute("src", "data:image/png;base64,iVBORw0KGgo=");
    expect(screen.getByText("published-r")).toBeInTheDocument();
  });
  it("withheld post-operation output is unknown; arbitrary user-state errors are not operation errors", () => {
    const { unmount } = render(
      <AppTool
        toolCall={call}
        result={receipt(
          "App operation completed, but its response was withheld by post-operation privacy clearance: private",
          true,
        )}
      />,
    );
    expect(screen.getByTestId("transcript-status-pill")).toHaveTextContent(
      "Response withheld · outcome unknown",
    );
    expect(screen.queryByTestId("transcript-meta")).toBeNull();
    unmount();
    render(
      <AppTool
        toolCall={{
          ...call,
          function: { name: "app_state", arguments: '{"action":"set"}' },
        }}
        result={receipt(
          '{"revision":2,"value":{"error":"my note","status":"failed"}}',
        )}
      />,
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "Result recorded",
    );
  });
});
