import { describe, expect, test } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { Theme } from "@radix-ui/themes";

import { ChromeTool } from "./ChromeTool";
import { browserSlice } from "../../../features/Browser/browserSlice";
import { reducer as configReducer } from "../../../features/Config/configSlice";
import type { ToolCall } from "../../../services/refact/types";

function makeStore(toolMessage: {
  tool_call_id: string;
  content: string | { m_type: string; m_content: string }[];
  tool_failed?: boolean;
}) {
  return configureStore({
    reducer: {
      browser: browserSlice.reducer,
      config: configReducer,
      chat: (
        state = {
          current_thread_id: "chat-1",
          threads: {
            "chat-1": {
              thread: {
                messages: [
                  {
                    role: "tool",
                    tool_call_id: toolMessage.tool_call_id,
                    content: toolMessage.content,
                    tool_failed: toolMessage.tool_failed,
                  },
                ],
              },
            },
          },
        },
      ) => state,
    },
  });
}

describe("ChromeTool", () => {
  test("renders typed browser request and execution report", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-1",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: {
            session: "shared_default",
            target: { type: "active" },
            steps: [
              { action: "navigate", url: "https://example.com" },
              {
                action: "fill",
                locator: { by: "css", value: "input[name=q]" },
                text: "hello",
              },
            ],
          },
        }),
      },
    };

    const store = makeStore({
      tool_call_id: "tc-1",
      content: JSON.stringify({
        ok: true,
        steps: [
          {
            step_index: 0,
            ok: true,
            summary: "Navigated to https://example.com",
            retries: 0,
          },
          {
            step_index: 1,
            ok: true,
            summary: "Filled <input> with 5 chars",
            fill_strategy: "dom_value_setter",
            field_kind: "text_input",
            verified: true,
            retries: 1,
          },
        ],
        url: "https://example.com",
        title: "Example",
      }),
    });

    render(
      <Provider store={store}>
        <Theme>
          <ChromeTool toolCall={toolCall} />
        </Theme>
      </Provider>,
    );

    await user.click(screen.getByText(/Browser action/i));

    expect(screen.getByText(/Browser action/i)).toBeInTheDocument();
    expect(screen.getByText("Request")).toBeInTheDocument();
    expect(screen.getByText("Results")).toBeInTheDocument();
    expect(screen.getByText("Execution Report")).toBeInTheDocument();
    expect(screen.getAllByText((text) => text.includes("Navigated to https://example.com")).length).toBeGreaterThan(0);
    expect(screen.getAllByText((text) => text.includes("Filled <input> with 5 chars")).length).toBeGreaterThan(0);
  });

  test("falls back to legacy command summary and text log", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-2",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          commands: "navigate_to 1 https://example.com\nscreenshot 1",
        }),
      },
    };

    const store = makeStore({
      tool_call_id: "tc-2",
      content: [
        { m_type: "text", m_content: "Navigated to https://example.com" },
        { m_type: "image/jpeg", m_content: "/9j/4AAQSkZJRgABAQAAAQABAAD/2w==" },
      ],
    });

    render(
      <Provider store={store}>
        <Theme>
          <ChromeTool toolCall={toolCall} />
        </Theme>
      </Provider>,
    );

    await user.click(screen.getByText(/Browser/i));

    expect(screen.getByText(/Browser/i)).toBeInTheDocument();
    expect(screen.getAllByText(/example.com/).length).toBeGreaterThan(0);
    expect(screen.getByText(/1 screenshot/)).toBeInTheDocument();
  });
});
