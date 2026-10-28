import { describe, expect, it } from "vitest";
import {
  createDefaultChatState,
  fireEvent,
  render,
  screen,
  within,
} from "../../../utils/test-utils";
import type {
  ChatMessages,
  MultiModalToolContent,
  ToolCall,
} from "../../../services/refact/types";
import { ToolContent } from "../ToolsContent";

const PNG = "iVBORw0KGgo=";

function call(id: string, name: string, args: Record<string, unknown>) {
  return {
    id,
    index: 0,
    type: "function",
    function: { name, arguments: JSON.stringify(args) },
  } satisfies ToolCall;
}

function renderCalls(
  calls: ToolCall[],
  contents: Record<string, MultiModalToolContent[]>,
) {
  const chat = createDefaultChatState();
  chat.threads[chat.current_thread_id].thread.messages = calls.map((item) => ({
    role: "tool",
    tool_call_id: item.id ?? "",
    content: contents[item.id ?? ""],
    tool_failed: false,
  })) as ChatMessages;
  return render(<ToolContent toolCalls={calls} />, {
    preloadedState: { chat },
  });
}

describe("MultiModalTool", () => {
  it("renders one transcript row per call with no legacy group header", () => {
    const first = call("mm-1", "take_screenshot", { url: "https://a.test" });
    const second = call("mm-2", "take_screenshot", { url: "https://b.test" });
    const { container } = renderCalls([first, second], {
      "mm-1": [{ m_type: "image/png", m_content: PNG }],
      "mm-2": [
        { m_type: "text", m_content: "second capture" },
        { m_type: "image/png", m_content: PNG },
        { m_type: "image/jpeg", m_content: PNG },
      ],
    });
    const rows = screen.getAllByTestId("multimodal-tool");
    expect(rows).toHaveLength(2);
    expect(container.querySelectorAll("[data-transcript-item]").length).toBe(2);
    expect(container.textContent).not.toMatch(/Take screenshot \(2\)/);
    expect(container.querySelector("pre code.language-python")).toBeNull();
    expect(within(rows[0]).getByTestId("transcript-verb")).toHaveTextContent(
      "Take screenshot",
    );
    expect(within(rows[0]).getByTestId("transcript-meta")).toHaveTextContent(
      "1 image",
    );
    expect(within(rows[1]).getByTestId("transcript-meta")).toHaveTextContent(
      "2 images",
    );
    expect(within(rows[0]).queryByTestId("transcript-status-pill")).toBeNull();
  });

  it("opens by default when images exist and shows images, args and text in sections", () => {
    const item = call("mm-open", "capture_page", {
      selector: "#main",
      full_page: true,
    });
    const { container } = renderCalls([item], {
      "mm-open": [
        { m_type: "text", m_content: "Captured the main region" },
        { m_type: "image/png", m_content: "<omitted>" },
        { m_type: "image/png", m_content: PNG },
      ],
    });
    const row = screen.getByTestId("multimodal-tool");
    expect(row).toHaveAttribute("data-open", "true");
    const sections = Array.from(
      container.querySelectorAll("[data-section]"),
    ).map((node) => node.getAttribute("data-section"));
    expect(sections).toEqual(["input", "output", "artifacts", "raw"]);
    const input = container.querySelector('[data-section="input"]');
    expect(input).toHaveTextContent("selector");
    expect(input).toHaveTextContent("#main");
    expect(input?.textContent).not.toContain("{");
    expect(
      container.querySelector('[data-section="output"]'),
    ).toHaveTextContent("Captured the main region");
    const artifacts = container.querySelector('[data-section="artifacts"]');
    const sources = Array.from(artifacts?.querySelectorAll("img") ?? []).map(
      (image) => image.getAttribute("src"),
    );
    expect(sources).toEqual([`data:image/png;base64,${PNG}`]);
  });

  it("stays closed without images and keeps raw arguments and response", () => {
    const item = call("mm-text", "describe_page", { url: "https://c.test" });
    renderCalls([item], {
      "mm-text": [
        { m_type: "text", m_content: "only text" },
        { m_type: "image/png", m_content: "<omitted>" },
      ],
    });
    const row = screen.getByTestId("multimodal-tool");
    expect(row).toHaveAttribute("data-open", "false");
    expect(within(row).queryByTestId("transcript-meta")).toBeNull();
    fireEvent.click(screen.getByTestId("transcript-toggle"));
    fireEvent.click(screen.getByRole("button", { name: /^Raw/ }));
    const raw = screen.getByTestId("transcript-raw");
    expect(raw).toHaveTextContent("https://c.test");
    expect(raw).toHaveTextContent("only text");
  });
});
