import { describe, expect, test } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { Theme } from "@radix-ui/themes";

import { WebTool } from "./WebTool";
import { reducer as configReducer } from "../../../features/Config/configSlice";
import type { ToolCall } from "../../../services/refact/types";

function makeStore(toolMessage: {
  tool_call_id: string;
  content: string;
  tool_failed?: boolean;
  extra?: Record<string, unknown>;
}) {
  return configureStore({
    reducer: {
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
                    extra: toolMessage.extra,
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

describe("WebTool", () => {
  test("renders structured web search results from tool message extra", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-search-1",
      index: 0,
      function: {
        name: "web_search",
        arguments: JSON.stringify({
          query: "rust async tutorial",
          num_results: 3,
        }),
      },
    };

    const store = makeStore({
      tool_call_id: "tc-search-1",
      content:
        'Web search results for "rust async tutorial":\n\n1. [Old Text Only](https://example.com)\n   This should be ignored in favor of structured data.\n',
      extra: {
        search_results: [
          {
            title: "Async Programming in Rust",
            url: "https://rust-lang.github.io/async-book/",
            snippet: "The official async book for Rust.",
          },
          {
            title: "Tokio Tutorial",
            url: "https://tokio.rs/tokio/tutorial",
            snippet: "Build async apps with Tokio.",
          },
        ],
      },
    });

    render(
      <Provider store={store}>
        <Theme>
          <WebTool toolCall={toolCall} toolType="web_search" />
        </Theme>
      </Provider>,
    );

    await user.click(screen.getByText(/Search web/i));

    expect(screen.getByText(/Results \(2\)/i)).toBeInTheDocument();
    expect(screen.getByText("Async Programming in Rust")).toBeInTheDocument();
    expect(
      screen.getByText("https://rust-lang.github.io/async-book/"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("The official async book for Rust."),
    ).toBeInTheDocument();
    expect(screen.queryByText("Old Text Only")).not.toBeInTheDocument();
  });

  test("falls back to parsing markdown-style text results", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-search-2",
      index: 0,
      function: {
        name: "web_search",
        arguments: JSON.stringify({ query: "example search" }),
      },
    };

    const store = makeStore({
      tool_call_id: "tc-search-2",
      content:
        'Web search results for "example search":\n\n1. [Example](https://example.com)\n   Example snippet\n\n2. [Docs](https://docs.example.com)\n   Another snippet\n',
    });

    render(
      <Provider store={store}>
        <Theme>
          <WebTool toolCall={toolCall} toolType="web_search" />
        </Theme>
      </Provider>,
    );

    await user.click(screen.getByText(/Search web/i));

    expect(screen.getByText(/Results \(2\)/i)).toBeInTheDocument();
    expect(screen.getByText("Example")).toBeInTheDocument();
    expect(screen.getByText("https://example.com")).toBeInTheDocument();
    expect(screen.getByText("Example snippet")).toBeInTheDocument();
  });
});
