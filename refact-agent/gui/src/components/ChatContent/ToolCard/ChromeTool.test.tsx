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
  test("summarizes coverage and virtual authenticator steps", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-instrumentation",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: {
            steps: [
              { action: "start_coverage", js: true, css: true },
              { action: "stop_coverage" },
              {
                action: "add_credential",
                id: "auth-1",
                credential: {
                  credential_id: "secret-id",
                  private_key: "secret-key",
                },
              },
              { action: "list_credentials", id: "auth-1" },
            ],
          },
        }),
      },
    };
    const store = makeStore({
      tool_call_id: "tc-instrumentation",
      content: JSON.stringify({ ok: true, steps: [] }),
    });

    const view = render(
      <Provider store={store}>
        <Theme>
          <ChromeTool toolCall={toolCall} />
        </Theme>
      </Provider>,
    );

    expect(screen.getByText(/Start JS\/CSS coverage/i)).toBeInTheDocument();
    expect(screen.getByText(/Stop JS\/CSS coverage/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Add authenticator credential/i),
    ).toBeInTheDocument();
    await user.click(screen.getByText(/Browser action/i));
    expect(
      screen.getByText((text) => text.includes('"action": "list_credentials"')),
    ).toBeInTheDocument();
    expect(view.container.textContent).toContain("[REDACTED]");
    expect(view.container.textContent).not.toMatch(/secret-key|secret-id/);
  });

  test("summarizes drag, file drop, and coordinate mouse steps", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-pointer-actions",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: {
            steps: [
              {
                action: "drag_and_drop",
                source: { by: "ref", value: "e1" },
                target: { by: "ref", value: "e2" },
              },
              {
                action: "drop_files",
                target: { by: "ref", value: "e2" },
                paths: ["/tmp/a.txt", "/tmp/b.txt"],
              },
              { action: "mouse_click_xy", x: 125, y: 240 },
            ],
          },
        }),
      },
    };
    const store = makeStore({
      tool_call_id: "tc-pointer-actions",
      content: JSON.stringify({
        ok: true,
        steps: [
          { step_index: 0, ok: true, summary: "Dragged element", retries: 0 },
          { step_index: 1, ok: true, summary: "Dropped files", retries: 0 },
          {
            step_index: 2,
            ok: true,
            summary: "Clicked coordinates",
            retries: 0,
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

    expect(screen.getByText(/Drag element to target/i)).toBeInTheDocument();
    expect(screen.getByText(/Drop 2 files/i)).toBeInTheDocument();
    expect(
      screen.getByText(/Mouse Click Xy \(125, 240\)/i),
    ).toBeInTheDocument();
    await user.click(screen.getByText(/Browser action/i));
    expect(screen.getByText("Results")).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("Dropped files")),
    ).toBeInTheDocument();
  });

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

    const artifactsTrigger = screen.getByRole("button", {
      name: "Artifacts — 1 screenshot, 1 PDF",
    });
    expect(artifactsTrigger).toHaveAttribute("aria-expanded", "false");
    await user.click(artifactsTrigger);
    expect(screen.getByText("320×200 · 1.2 KB")).toBeInTheDocument();
    const pdf = screen.getByRole("link", { name: /Open PDF page.pdf/i });
    expect(pdf).toHaveAttribute("href", "file:///tmp/refact-browser/page.pdf");
    expect(
      screen.getByText("/tmp/refact-browser/page.pdf"),
    ).toBeInTheDocument();
  });

  test("summarizes WebSocket and HAR actions and renders WebSocket reports", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-network-advanced",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: {
            steps: [
              {
                action: "route_web_socket",
                pattern: "wss://example.test/**",
                mode: "mock",
              },
              { action: "start_har_recording", mode: "full", content: "embed" },
              { action: "stop_har_recording" },
            ],
          },
        }),
      },
    };
    const store = makeStore({
      tool_call_id: "tc-network-advanced",
      content: JSON.stringify({
        ok: true,
        steps: [
          {
            step_index: 0,
            ok: true,
            summary: "Added WebSocket route",
            retries: 0,
          },
          {
            step_index: 1,
            ok: true,
            summary: "Started HAR recording",
            retries: 0,
          },
          { step_index: 2, ok: true, summary: "Saved HAR", retries: 0 },
        ],
        websockets: [
          {
            sequence: 1,
            socket_id: "ws-1",
            url: "wss://example.test/socket",
            kind: "frame_received",
            data: "masked frame",
            routed: true,
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

    expect(screen.getByText(/Route WebSocket/i)).toBeInTheDocument();
    expect(screen.getByText(/Start HAR recording/i)).toBeInTheDocument();
    await user.click(screen.getByText(/Browser action/i));
    expect(screen.getByText("WebSockets")).toBeInTheDocument();
    expect(screen.getByTestId("websocket-route")).toBeInTheDocument();
    expect(screen.getByText("mock")).toBeInTheDocument();
    expect(screen.getByText("wss://example.test/**")).toBeInTheDocument();
    expect(screen.getByText("received")).toBeInTheDocument();
    expect(screen.getByText("routed")).toBeInTheDocument();
    expect(screen.getByText("masked frame")).toBeInTheDocument();
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
    expect(
      screen.getByRole("button", { name: "Artifacts — 1 download" }),
    ).toBeInTheDocument();
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
    await user.click(
      screen.getByRole("button", { name: "Artifacts — 1 download" }),
    );
    expect(screen.getByText("browser-fixture.txt")).toBeInTheDocument();
    expect(screen.getByText("25 B")).toBeInTheDocument();
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

  test("renders the text-first page envelope, snapshot pointer and locator echoes", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-envelope",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: {
            steps: [
              { action: "navigate", url: "https://example.com/pricing" },
              { action: "click", locator: { by: "ref", value: "e1" } },
            ],
          },
        }),
      },
    };
    const store = makeStore({
      tool_call_id: "tc-envelope",
      content: JSON.stringify({
        ok: true,
        steps: [
          {
            step_index: 0,
            ok: true,
            summary: "Navigated to https://example.com/pricing",
            retries: 0,
          },
          {
            step_index: 1,
            ok: true,
            summary: "click on <button>",
            retries: 0,
            locator_echo: "getByRole('button', { name: 'Save' })",
          },
        ],
        url: "https://example.com/pricing",
        title: "Pricing",
        console: [{ timestamp: 1, level: "Error", text: "boom" }],
        page: {
          status: 404,
          console: { errors: 1, warnings: 2 },
          snapshot: {
            yaml: '- button "Save" [ref=e1]',
            lines: 812,
            bytes: 41231,
            truncated: true,
            artifact: {
              kind: "aria_snapshot",
              mime: "text/yaml",
              path: "/artifacts/snapshot-1.yaml",
              bytes: 41231,
            },
          },
        },
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

    expect(screen.getByTestId("browser-page-header")).toBeInTheDocument();
    expect(screen.getByText("https://example.com/pricing")).toBeInTheDocument();
    expect(screen.getByText("Pricing")).toBeInTheDocument();

    const status = screen.getByTestId("browser-page-status");
    expect(status).toHaveTextContent("HTTP 404");
    expect(status).toHaveAttribute("data-status", "error");

    const consoleChip = screen.getByTestId("browser-page-console");
    expect(consoleChip).toHaveTextContent("1 error · 2 warnings");
    expect(consoleChip).toHaveAttribute("aria-expanded", "false");
    expect(screen.queryByText("Console")).not.toBeInTheDocument();
    await user.click(consoleChip);
    expect(consoleChip).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("Console")).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("[Error] boom")),
    ).toBeInTheDocument();

    expect(screen.getByTestId("browser-step-rows")).toBeInTheDocument();
    const echo = screen.getByTestId("browser-locator-echo");
    expect(echo).toHaveTextContent("getByRole('button', { name: 'Save' })");
    expect(screen.getByText("click on <button>")).toBeInTheDocument();

    const snapshotTrigger = screen.getByRole("button", {
      name: "Page Snapshot — 812 lines · 40.3 KB",
    });
    expect(snapshotTrigger).toHaveAttribute("aria-expanded", "false");
    await user.click(snapshotTrigger);
    expect(screen.getByTestId("page-snapshot-truncated")).toHaveTextContent(
      "Truncated",
    );
    expect(screen.getByText("text/yaml")).toBeInTheDocument();
    expect(screen.getByText("/artifacts/snapshot-1.yaml")).toBeInTheDocument();
    expect(screen.getByText("button")).toBeInTheDocument();
    expect(screen.getByText("“Save”")).toBeInTheDocument();
    expect(screen.getByText("ref=e1")).toBeInTheDocument();
  });

  test("leaves pre-envelope payloads free of the new surfaces", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-legacy-envelope",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: { steps: [{ action: "navigate", url: "https://a.test" }] },
        }),
      },
    };
    const store = makeStore({
      tool_call_id: "tc-legacy-envelope",
      content: JSON.stringify({
        ok: true,
        steps: [
          {
            step_index: 0,
            ok: true,
            summary: "Navigated to https://a.test",
            retries: 0,
          },
        ],
        url: "https://a.test",
        title: "A",
        console: [{ timestamp: 1, level: "Log", text: "ready" }],
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

    expect(screen.queryByTestId("browser-page-header")).not.toBeInTheDocument();
    expect(screen.queryByTestId("browser-step-rows")).not.toBeInTheDocument();
    expect(
      screen.queryByTestId("browser-locator-echo"),
    ).not.toBeInTheDocument();
    expect(screen.queryByTestId("page-snapshot")).not.toBeInTheDocument();
    expect(screen.queryByText("Captures")).not.toBeInTheDocument();
    expect(screen.getByText("Results")).toBeInTheDocument();
    expect(screen.getByText("Console")).toBeInTheDocument();
    expect(
      screen.getByText((text) => text.includes("Navigated to https://a.test")),
    ).toBeInTheDocument();
  });

  test("labels a filmstrip capture with frame chips and one zoomable image", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-filmstrip",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: {
            steps: [{ action: "capture_frames", duration_ms: 900 }],
          },
        }),
      },
    };
    const store = makeStore({
      tool_call_id: "tc-filmstrip",
      content: [
        {
          m_type: "text",
          m_content: JSON.stringify({
            ok: true,
            steps: [
              {
                step_index: 0,
                ok: true,
                summary: "Captured 3 frame(s) over 900ms",
                retries: 0,
                data: {
                  mime: "image/jpeg",
                  data: "<omitted>",
                  width: 640,
                  height: 400,
                  bytes: 2048,
                  artifact: {
                    kind: "filmstrip",
                    mime: "image/jpeg",
                    path: "/artifacts/burst-filmstrip.jpg",
                    bytes: 2048,
                    width: 640,
                    height: 400,
                  },
                  frames: [
                    { index: 0, offset_ms: 0 },
                    { index: 1, offset_ms: 450, changed_percent: 12.5 },
                    { index: 2, offset_ms: 900, changed_percent: 3 },
                  ],
                  frame_count: 3,
                  columns: 3,
                  rows: 1,
                  duration_ms: 900,
                  warnings: ["captured with timed screenshots instead"],
                },
              },
            ],
          }),
        },
        { m_type: "image/jpeg", m_content: "/9j/4AAQSkZJRgABAQAAAQABAAD/2w==" },
      ],
    });

    const view = render(
      <Provider store={store}>
        <Theme>
          <ChromeTool toolCall={toolCall} />
        </Theme>
      </Provider>,
    );

    await user.click(screen.getByText(/Browser action/i));

    expect(screen.getByText("Captures")).toBeInTheDocument();
    expect(screen.getByTestId("browser-capture-filmstrip")).toBeInTheDocument();
    expect(screen.getByText("Filmstrip")).toBeInTheDocument();
    expect(screen.getByText("3 frames · 900ms")).toBeInTheDocument();
    expect(screen.getByText("+0ms")).toBeInTheDocument();
    expect(screen.getByText("+450ms · 12.5% changed")).toBeInTheDocument();
    expect(screen.getByText("+900ms · 3.0% changed")).toBeInTheDocument();
    expect(
      screen.getByText("captured with timed screenshots instead"),
    ).toBeInTheDocument();
    expect(screen.getByRole("img", { name: "Filmstrip" })).toBeInTheDocument();
    expect(view.container.querySelectorAll("img")).toHaveLength(1);
    expect(screen.getByTestId("capture-artifact")).toHaveTextContent(
      "/artifacts/burst-filmstrip.jpg",
    );
  });

  test("labels element galleries and state strips that carry no inline image", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-gallery",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: {
            steps: [
              { action: "screenshot_elements", compose: "grid" },
              { action: "capture_element_states" },
            ],
          },
        }),
      },
    };
    const store = makeStore({
      tool_call_id: "tc-gallery",
      content: JSON.stringify({
        ok: true,
        steps: [
          {
            step_index: 0,
            ok: true,
            summary: "Composed 2 element screenshots into a grid",
            retries: 0,
            data: {
              compose: "grid",
              count: 2,
              labels: ["header", "footer"],
              artifact: {
                kind: "image",
                mime: "image/png",
                width: 320,
                height: 200,
                bytes: 1024,
              },
              images: [{ label: "grid", mime: "image/png", data: "<omitted>" }],
            },
          },
          {
            step_index: 1,
            ok: true,
            summary: "Captured <button> in 3 states",
            retries: 0,
            data: {
              states: ["default", "hover", "focus"],
              artifact: {
                kind: "image",
                mime: "image/png",
                width: 320,
                height: 120,
                bytes: 512,
              },
              images: [
                { label: "states", mime: "image/png", data: "<omitted>" },
              ],
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

    expect(
      screen.getByTestId("browser-capture-element_gallery"),
    ).toBeInTheDocument();
    expect(screen.getByText("Element gallery")).toBeInTheDocument();
    expect(screen.getByText("2 elements · grid")).toBeInTheDocument();
    expect(
      screen.getByTestId("browser-capture-element_states"),
    ).toBeInTheDocument();
    expect(screen.getByText("Element states")).toBeInTheDocument();
    expect(screen.getByText("default · hover · focus")).toBeInTheDocument();
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

  test("renders wave-3 step families inside the card", async () => {
    const user = userEvent.setup();
    const toolCall: ToolCall = {
      id: "tc-families",
      index: 0,
      function: {
        name: "chrome",
        arguments: JSON.stringify({
          request: {
            steps: [
              { action: "fast_forward", ticks_ms: 3_600_000 },
              { action: "list_routes" },
              { action: "http_request", url: "https://api.example.test/ping" },
              { action: "cdp_send", method: "Page.enable" },
              { action: "list_devices" },
              { action: "reset" },
              { action: "count" },
            ],
          },
        }),
      },
    };
    const store = makeStore({
      tool_call_id: "tc-families",
      content: JSON.stringify({
        ok: true,
        network_summary: ["GET https://example.test/app.js 200 2048b 40ms"],
        steps: [
          {
            step_index: 0,
            ok: true,
            summary: "Fast-forwarded clock",
            retries: 0,
            data: { clock: { installed: true, paused: false } },
          },
          {
            step_index: 1,
            ok: true,
            summary: "Listed 1 network route(s)",
            retries: 0,
            data: {
              routes: [
                {
                  order: 0,
                  pattern: "**/api/**",
                  handler: { type: "abort", reason: "blockedbyclient" },
                },
              ],
            },
          },
          {
            step_index: 2,
            ok: true,
            summary: "GET https://api.example.test/ping -> 204",
            retries: 0,
            data: {
              http_request: {
                method: "GET",
                url: "https://api.example.test/ping",
                status: 204,
                body_bytes: 0,
              },
            },
          },
          {
            step_index: 3,
            ok: true,
            summary: "Page.enable on page returned 12 bytes",
            retries: 0,
            data: {
              cdp_send: {
                method: "Page.enable",
                target: "page",
                warnings: [],
                bytes: 12,
                result: {},
              },
            },
          },
          {
            step_index: 4,
            ok: true,
            summary: "2 matching device(s)",
            retries: 0,
            data: { devices: ["Pixel 7", "iPad Pro 11"], aliases: ["mobile"] },
          },
          {
            step_index: 5,
            ok: true,
            summary: "Reset browser state",
            retries: 0,
            data: {
              reset: {
                routes: 1,
                har_replays: 0,
                websocket_routes: 0,
                locator_handlers: 0,
                authenticators: 0,
                init_scripts: 0,
                offline: false,
                throttling_cleared: true,
                emulation_cleared: true,
                clock_cleared: true,
                service_worker_block_cleared: false,
              },
            },
          },
          {
            step_index: 6,
            ok: true,
            summary: "Matched 3 element(s)",
            retries: 0,
            data: { count: 3 },
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

    expect(screen.getByTestId("network-summary")).toBeInTheDocument();
    expect(screen.getByTestId("route-chain")).toBeInTheDocument();
    expect(screen.getByTestId("http-requests")).toBeInTheDocument();
    expect(screen.getByTestId("clock-timeline")).toBeInTheDocument();
    expect(screen.getByTestId("readouts")).toBeInTheDocument();
    expect(screen.getByTestId("devices")).toBeInTheDocument();
    expect(screen.getByTestId("cdp")).toBeInTheDocument();
    expect(screen.getByTestId("reset")).toBeInTheDocument();
    expect(screen.getByText("fast_forward +01:00")).toBeInTheDocument();
    expect(screen.getByText("3 matched")).toBeInTheDocument();
    expect(screen.getByText("1 routes")).toBeInTheDocument();
  });
});
