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
  test("renders image and PDF artifacts", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-artifacts",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: {
            steps: [{ action: "screenshot" }, { action: "pdf" }],
          },
        }),
      },
    };
    const store = makeStore({
      tool_call_id: "tc-artifacts",
      content: JSON.stringify({
        ok: true,
        steps: [
          {
            step_index: 0,
            ok: true,
            summary: "Screenshot captured",
            retries: 0,
            data: {
              artifact: {
                kind: "image",
                mime: "image/png",
                data: "aW1hZ2U=",
                width: 320,
                height: 200,
                bytes: 1234,
              },
            },
          },
          {
            step_index: 1,
            ok: true,
            summary: "PDF saved",
            retries: 0,
            data: {
              artifact: {
                kind: "pdf",
                mime: "application/pdf",
                path: "/tmp/refact-browser/page.pdf",
                bytes: 4096,
                data: null,
              },
            },
          },
        ],
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

    expect(screen.getByText("Artifacts")).toBeInTheDocument();
    expect(screen.getByText("320×200 · 1234 B")).toBeInTheDocument();
    const pdf = screen.getByRole("link", { name: /PDF · 4096 B/i });
    expect(pdf).toHaveAttribute("href", "file:///tmp/refact-browser/page.pdf");
    expect(
      screen.getByText("/tmp/refact-browser/page.pdf"),
    ).toBeInTheDocument();
  });

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
        stabilized: false,
        console: [
          { timestamp: 1, level: "Log", text: "loaded transactional state" },
        ],
        page_errors: ["ReferenceError: fixtureFailure is not defined"],
        network: [
          {
            timestamp: 2,
            method: "GET",
            url: "https://example.com/api/items",
            resource_type: "Fetch",
            status: 404,
            status_text: "Not Found",
            request_headers: {},
            response_headers: {},
            transfer_size: 321,
            failure_text: "net::ERR_HTTP_RESPONSE_CODE_FAILURE",
            from_service_worker: false,
            is_navigation_request: false,
          },
        ],
        context: {
          viewport: "390x844 @3x mobile touch",
          locale: "ja-JP",
          timezone: "Asia/Tokyo",
          color_scheme: "dark",
          permissions: ["geolocation"],
          cookie_count: 2,
          local_storage_count: 1,
          session_storage_count: 0,
          offline: false,
          http_credentials: true,
        },
        locator_handlers: [
          {
            name: "dismiss_overlays",
            action: "click",
            outcome: "Cookie banner dismissed",
            ok: true,
          },
          {
            name: "close_interstitial",
            action: "press Escape",
            outcome: "Interstitial remained visible",
            ok: false,
          },
        ],
        dialogs: [
          {
            type: "prompt",
            message: "Name for fixture?",
            default_value: "visitor",
            action: "accepted",
            automatic: false,
          },
          {
            type: "confirm",
            message: "Continue with fixture?",
            default_value: "",
            action: "dismissed",
            automatic: true,
          },
        ],
        uploads: [
          {
            paths: ["/workspace/fixture.txt"],
            source: "direct",
            in_memory_payloads: false,
          },
        ],
        downloads: [
          {
            guid: "download-guid",
            url: "https://example.com/browser-fixture.txt",
            frame_id: "frame",
            suggested_filename: "browser-fixture.txt",
            local_path: "/runtime/download-guid",
            received_bytes: 25,
            total_bytes: 25,
            state: "completed",
          },
        ],
        new_tabs: [
          {
            id: "popup-target",
            target_id: "popup-target",
            url: "https://example.com/popup",
            title: "Popup Fixture",
            active: true,
            opener: { tab_id: "primary-target", frame_id: "main-frame" },
            opened_by_step: 1,
          },
        ],
        active_routes: [
          {
            pattern: "**/api/**",
            handler: { type: "fulfill", status: 200 },
          },
        ],
        intercepted_requests: [
          {
            url: "https://example.com/api/data",
            method: "GET",
            pattern: "**/api/**",
            action: "fulfill",
            status: 200,
            redirect_hop: true,
          },
        ],
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
    expect(screen.getByText("Page State")).toBeInTheDocument();
    expect(screen.getByText("Console")).toBeInTheDocument();
    expect(screen.getByText("Page Errors")).toBeInTheDocument();
    expect(screen.getByText("Context")).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("390x844 @3x mobile touch")),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("cookies: 2")),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("HTTP credentials: configured")),
    ).toBeInTheDocument();
    const networkTrigger = screen.getByRole("button", { name: "Network (1)" });
    expect(networkTrigger).toHaveAttribute("aria-expanded", "false");
    await user.click(networkTrigger);
    expect(screen.getByText("Locator Handlers")).toBeInTheDocument();
    expect(screen.getByText("Dialogs")).toBeInTheDocument();
    expect(screen.getByText("Uploads")).toBeInTheDocument();
    expect(screen.getByText("Downloads")).toBeInTheDocument();
    expect(screen.getByText("New Tabs")).toBeInTheDocument();
    expect(screen.getByText("Active Routes")).toBeInTheDocument();
    expect(screen.getByText("Intercepted Requests")).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("fulfill: **/api/**")),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) =>
        text.includes("GET https://example.com/api/data"),
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("Popup Fixture")),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("opener primary-target")),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("DOM stabilized: No")),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("loaded transactional state")),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) =>
        text.includes("fixtureFailure is not defined"),
      ),
    ).toBeInTheDocument();
    expect(screen.getByText("GET")).toBeInTheDocument();
    expect(screen.getByLabelText("Status 404")).toBeInTheDocument();
    expect(
      screen.getByTitle("https://example.com/api/items"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("net::ERR_HTTP_RESPONSE_CODE_FAILURE"),
    ).toBeInTheDocument();
    expect(screen.getByText("dismiss_overlays")).toBeInTheDocument();
    expect(screen.getByText("Action: click")).toBeInTheDocument();
    expect(
      screen.getByText("Outcome: Cookie banner dismissed"),
    ).toBeInTheDocument();
    expect(screen.getByText("close_interstitial")).toBeInTheDocument();
    expect(screen.getByText("Action: press Escape")).toBeInTheDocument();
    expect(
      screen.getByText("Outcome: Interstitial remained visible"),
    ).toBeInTheDocument();
    expect(screen.getByText("Succeeded").parentElement).toHaveAttribute(
      "data-status",
      "success",
    );
    expect(screen.getByText("Failed").parentElement).toHaveAttribute(
      "data-status",
      "error",
    );
    expect(
      screen.getByText((text) =>
        text.includes(
          "[prompt] accepted: Name for fixture? (default: visitor)",
        ),
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) =>
        text.includes("[confirm] auto-dismissed: Continue with fixture?"),
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("/workspace/fixture.txt")),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("browser-fixture.txt · 25 B")),
    ).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("/runtime/download-guid")),
    ).toBeInTheDocument();
    expect(screen.queryByText("Execution Report")).not.toBeInTheDocument();
    expect(screen.queryByText("ARIA Snapshot")).not.toBeInTheDocument();
    expect(
      screen.getAllByText((text) =>
        text.includes("Navigated to https://example.com"),
      ).length,
    ).toBeGreaterThan(0);
    expect(
      screen.getAllByText((text) =>
        text.includes("Filled <input> with 5 chars"),
      ).length,
    ).toBeGreaterThan(0);
  });

  test("renders ARIA snapshot step data as a structured tree", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-aria-snapshot",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: { steps: [{ action: "snapshot" }] },
        }),
      },
    };
    const store = makeStore({
      tool_call_id: "tc-aria-snapshot",
      content: JSON.stringify({
        ok: true,
        steps: [
          {
            step_index: 0,
            ok: true,
            summary: "ARIA snapshot captured",
            retries: 0,
            data: {
              yaml: '- button "Save" [ref=e1]',
              nodes: [{ role: "button", name: "Save", ref: "e1" }],
              generation: {
                document_generation: 1,
                frame_generation: 1,
                refs: {},
              },
            },
          },
        ],
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

    expect(screen.getByText("ARIA Snapshot")).toBeInTheDocument();
    expect(screen.getByText("button")).toBeInTheDocument();
    expect(screen.getByText("“Save”")).toBeInTheDocument();
    expect(screen.getByText("ref=e1")).toBeInTheDocument();
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
    expect(screen.queryByText("Locator Handlers")).not.toBeInTheDocument();
  });

  test("renders actionability diagnostics for a failed typed step", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-actionability",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: {
            steps: [
              {
                action: "click",
                locator: { by: "role", value: "button" },
              },
            ],
          },
        }),
      },
    };
    const store = makeStore({
      tool_call_id: "tc-actionability",
      tool_failed: true,
      content: JSON.stringify({
        ok: false,
        steps: [
          {
            step_index: 0,
            ok: false,
            summary: "Click failed",
            error: "timed out waiting for element to be stable",
            retries: 3,
            actionability: {
              call_log: [
                "waiting for locator",
                "element is not stable",
                "retrying click action",
              ],
              timed_out: true,
              elapsed_ms: 5031,
              attempts: 4,
              attached: true,
              stable: false,
            },
          },
        ],
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

    expect(screen.getByText("Actionability")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Step 1 actionability" }),
    ).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("retrying click action")).toBeInTheDocument();
    expect(screen.getByTestId("actionability-state-editable")).toHaveAttribute(
      "data-result",
      "not-checked",
    );
  });

  test("renders assertion pass failure values and ARIA diff", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-assertions",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: {
            steps: [
              {
                action: "expect",
                matcher: { type: "to_have_title", expected: "Dashboard" },
              },
              {
                action: "expect",
                locator: { by: "role", role: "navigation" },
                matcher: {
                  type: "to_match_aria_snapshot",
                  expected: '- navigation "Primary"',
                },
                soft: true,
              },
            ],
          },
        }),
      },
    };
    const store = makeStore({
      tool_call_id: "tc-assertions",
      content: JSON.stringify({
        ok: true,
        steps: [
          {
            step_index: 0,
            ok: true,
            summary: "Assertion passed: to_have_title",
            retries: 1,
            assertion: {
              matcher: "to_have_title",
              passed: true,
              soft: false,
              expected: "Dashboard",
              received: "Dashboard",
              attempts: 2,
              elapsed_ms: 20,
            },
          },
          {
            step_index: 1,
            ok: false,
            summary: "Soft assertion failed: to_match_aria_snapshot",
            error: "Expected snapshot did not match",
            retries: 3,
            assertion: {
              matcher: "to_match_aria_snapshot",
              passed: false,
              soft: true,
              expected: '- navigation "Primary"',
              received: '- navigation "Secondary"',
              diff: '--- expected\n+++ received\n- navigation "Primary"\n+ navigation "Secondary"',
              attempts: 4,
              elapsed_ms: 170,
            },
          },
        ],
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

    expect(screen.getByText("Assertions")).toBeInTheDocument();
    expect(
      screen.getByText("Passed").parentElement?.parentElement,
    ).toHaveAttribute("data-status", "success");
    expect(
      screen.getByText("Failed").parentElement?.parentElement,
    ).toHaveAttribute("data-status", "error");
    expect(screen.getByText("Soft")).toBeInTheDocument();
    expect(screen.getAllByText("Expected")).toHaveLength(2);
    expect(screen.getAllByText("Received")).toHaveLength(2);
    expect(screen.getByText("Diff")).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes('+ navigation "Secondary"')),
    ).toBeInTheDocument();
  });
});
