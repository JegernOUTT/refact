import { describe, expect, test } from "vitest";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Provider } from "react-redux";
import { configureStore } from "@reduxjs/toolkit";
import { Theme } from "@radix-ui/themes";

import { ChromeTool } from "./ChromeTool";
import { browserSlice } from "../../../features/Browser/browserSlice";
import { reducer as configReducer } from "../../../features/Config/configSlice";
import type { ToolCall } from "../../../services/refact/types";

type ToolContent = string | { m_type: string; m_content: string }[];

function makeStore(id: string, content: ToolContent) {
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
                messages: [{ role: "tool", tool_call_id: id, content }],
              },
            },
          },
        },
      ) => state,
    },
  });
}

function renderChrome(args: unknown, content: ToolContent, resultId?: string) {
  const id = "tc-chrome";
  const toolCall: ToolCall = {
    id,
    index: 0,
    function: { name: "chrome", arguments: JSON.stringify(args) },
  };
  return render(
    <Provider store={makeStore(resultId ?? id, content)}>
      <Theme>
        <ChromeTool toolCall={toolCall} />
      </Theme>
    </Provider>,
  );
}

async function expand() {
  await userEvent.setup().click(screen.getByTestId("transcript-toggle"));
}

function section(id: string): HTMLElement {
  const node = document.querySelector<HTMLElement>(`[data-section="${id}"]`);
  if (!node) throw new Error(`missing section ${id}`);
  return node;
}

async function openRaw() {
  await userEvent
    .setup()
    .click(within(section("raw")).getByRole("button", { name: "Raw" }));
}

const PAGE_REPORT = {
  ok: false,
  url: "https://example.com/pricing",
  title: "Pricing",
  context: {
    viewport: "390x844 @3x mobile touch",
    cookie_count: 2,
    local_storage_count: 0,
    session_storage_count: 0,
    offline: false,
    http_credentials: true,
  },
  console: [
    { timestamp: 1, level: "error", text: "boom" },
    { timestamp: 2, level: "log", text: "hello" },
  ],
  network: [
    {
      timestamp: 1,
      method: "GET",
      url: "https://example.com/api/items",
      resource_type: "fetch",
      status: 404,
    },
  ],
  page: {
    status: 200,
    console: { errors: 2, warnings: 1 },
    snapshot: {
      yaml: '- button "Save" [ref=e1]',
      lines: 1,
      bytes: 24,
      truncated: true,
      artifact: {
        kind: "snapshot",
        mime: "text/yaml",
        path: "/artifacts/snapshot-1.yaml",
        bytes: 99,
      },
    },
  },
  steps: [
    { step_index: 0, ok: true, summary: "Navigated", retries: 0 },
    {
      step_index: 1,
      ok: true,
      summary: "click on <button>",
      retries: 0,
      locator_echo: "getByRole('button', { name: 'Save' })",
    },
    {
      step_index: 2,
      ok: false,
      summary: "fill input",
      error: "element is disabled",
      retries: 1,
    },
  ],
};

const PAGE_ARGS = {
  request: {
    steps: [
      { action: "navigate", url: "https://example.com/pricing" },
      { action: "click", locator: { by: "role", value: "button" } },
      { action: "fill", locator: { by: "label", value: "Email" } },
      { action: "screenshot" },
    ],
  },
};

describe("ChromeTool", () => {
  test("renders one row with a readable step target and meta", () => {
    renderChrome(PAGE_ARGS, JSON.stringify(PAGE_REPORT));
    expect(screen.getAllByTestId("transcript-row")).toHaveLength(1);
    expect(screen.getByTestId("transcript-verb")).toHaveTextContent("Browser");
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Navigate, click, fill · +1",
    );
    expect(screen.getByTestId("transcript-meta")).toHaveTextContent(
      "example.com · Pricing · 2 console errors",
    );
    expect(screen.getByTestId("chrome-tool")).toHaveAttribute(
      "data-transcript-status",
      "failed",
    );
    expect(screen.getByTestId("transcript-toggle")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
  });

  test("describes a single navigate step and stays quiet on success", () => {
    renderChrome(
      { request: { steps: [{ action: "navigate", url: "https://a.test/" }] } },
      JSON.stringify({ ok: true, steps: [] }),
    );
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Navigate https://a.test/",
    );
    expect(screen.getByTestId("chrome-tool")).toHaveAttribute(
      "data-transcript-status",
      "done",
    );
  });

  test("lays out summary, input, output, error and raw sections", async () => {
    renderChrome(PAGE_ARGS, JSON.stringify(PAGE_REPORT));
    await expand();

    const summary = section("summary");
    expect(summary).toHaveTextContent("Pricing");
    expect(summary).toHaveTextContent("https://example.com/pricing");
    expect(summary).toHaveTextContent("390x844 @3x mobile touch");
    expect(summary).toHaveTextContent("2/3 ok");
    expect(summary).toHaveTextContent("2 errors · 1 warning");

    const input = section("input");
    expect(input).toHaveTextContent("Navigate https://example.com/pricing");
    expect(input).toHaveTextContent("Fill label=Email");
    expect(input.textContent).not.toContain('"action"');

    const output = section("output");
    expect(within(output).getByTestId("browser-step-rows")).toHaveTextContent(
      "click on <button>",
    );
    expect(
      within(output).getByTestId("browser-locator-echo"),
    ).toHaveTextContent("getByRole('button', { name: 'Save' })");
    expect(output).toHaveTextContent("HTTP credentials: configured");

    const error = section("error");
    expect(error).toHaveTextContent("Step 3: fill input — element is disabled");
    expect(error).toHaveTextContent("Console: boom");
    expect(error.textContent).not.toContain("hello");

    await openRaw();
    const raw = section("raw");
    expect(raw).toHaveTextContent("Request");
    expect(raw).toHaveTextContent("Console log");
    expect(raw).toHaveTextContent("Network log");
    expect(raw).toHaveTextContent("/artifacts/snapshot-1.yaml");
    expect(raw).toHaveTextContent('button "Save"');
  });

  test("redacts secrets in the raw request", async () => {
    const view = renderChrome(
      {
        request: {
          steps: [
            { action: "start_coverage", js: true, css: true },
            {
              action: "add_credential",
              credential: { credential_id: "secret-id", private_key: "k" },
            },
          ],
        },
      },
      JSON.stringify({ ok: true, steps: [] }),
    );
    await expand();
    expect(section("input")).toHaveTextContent("Start JS/CSS coverage");
    expect(section("input")).toHaveTextContent("Add authenticator credential");
    await openRaw();
    expect(view.container.textContent).toContain("[REDACTED]");
    expect(view.container.textContent).not.toMatch(/secret-id/);
  });

  test("puts screenshots in artifacts and skips omitted images", async () => {
    renderChrome({ request: { steps: [{ action: "screenshot" }] } }, [
      {
        m_type: "text",
        m_content: JSON.stringify({
          ok: true,
          steps: [{ step_index: 0, ok: true, summary: "shot", retries: 0 }],
          screenshot: { mime: "image/png", data: "<omitted>" },
        }),
      },
      { m_type: "image/png", m_content: "<omitted>" },
      { m_type: "image/png", m_content: "iVBORw0KGgo=" },
    ]);
    await expand();
    const grid = within(section("artifacts")).getByTestId(
      "browser-screenshots",
    );
    const sources = Array.from(grid.querySelectorAll("img")).map((img) =>
      img.getAttribute("src"),
    );
    expect(sources).toEqual(["data:image/png;base64,iVBORw0KGgo="]);
  });

  test("labels filmstrip captures with frame chips", async () => {
    renderChrome({ request: { steps: [{ action: "capture_frames" }] } }, [
      {
        m_type: "text",
        m_content: JSON.stringify({
          ok: true,
          steps: [
            {
              step_index: 0,
              ok: true,
              summary: "Captured 2 frames",
              retries: 0,
              data: {
                mime: "image/jpeg",
                data: "<omitted>",
                artifact: { kind: "filmstrip", path: "/artifacts/burst.jpg" },
                frames: [
                  { index: 0, offset_ms: 0 },
                  { index: 1, offset_ms: 450, changed_percent: 12.5 },
                ],
                frame_count: 2,
              },
            },
          ],
        }),
      },
      { m_type: "image/jpeg", m_content: "/9j/4AAQ" },
    ]);
    await expand();
    const artifacts = section("artifacts");
    expect(
      within(artifacts).getByTestId("browser-capture-filmstrip"),
    ).toHaveTextContent("Filmstrip · 2 frames");
    expect(artifacts).toHaveTextContent("+450ms · 12.5% changed");
    expect(within(artifacts).getByTestId("capture-artifact")).toHaveTextContent(
      "/artifacts/burst.jpg",
    );
  });

  test("falls back to legacy commands and text log", async () => {
    renderChrome(
      { commands: "navigate_to 1 https://example.com\nscreenshot 1" },
      "legacy text log",
    );
    expect(screen.getByTestId("transcript-target")).toHaveTextContent(
      "Navigate https://example.com · screenshot",
    );
    await expand();
    expect(section("input")).toHaveTextContent(
      "navigate_to 1 https://example.com",
    );
    expect(section("output")).toHaveTextContent("legacy text log");
  });

  test("shows assertions and actionability as output blocks", async () => {
    renderChrome(
      { request: { steps: [{ action: "expect" }, { action: "click" }] } },
      JSON.stringify({
        ok: false,
        steps: [
          {
            step_index: 0,
            ok: true,
            summary: "assert",
            retries: 0,
            assertion: {
              matcher: "to_have_text",
              passed: true,
              soft: true,
              expected: "Hi",
              received: "Hi",
              attempts: 1,
              elapsed_ms: 5,
            },
          },
          {
            step_index: 1,
            ok: false,
            summary: "click",
            retries: 2,
            actionability: {
              call_log: ["retrying click action"],
              timed_out: true,
              editable: false,
            },
          },
        ],
      }),
    );
    await expand();
    const output = section("output");
    expect(output).toHaveTextContent("Assertions");
    expect(output).toHaveTextContent("To Have Text");
    expect(output).toHaveTextContent("Step 2 actionability");
    expect(output).toHaveTextContent("retrying click action");
    expect(output).toHaveTextContent("editable: no");
  });

  test("shows the running state until a result arrives", () => {
    renderChrome(PAGE_ARGS, "", "other-call");
    expect(screen.getByTestId("chrome-tool")).toHaveAttribute(
      "data-transcript-status",
      "running",
    );
  });
});
