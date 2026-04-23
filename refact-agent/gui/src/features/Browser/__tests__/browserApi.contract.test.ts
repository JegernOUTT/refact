import { describe, test, expect } from "vitest";
import type {
  BrowserActionRequest,
  BrowserActionResponse,
  BrowserStartResponse,
  BrowserStopResponse,
  BrowserScreenshotResponse,
  BrowserContextRequest,
  BrowserContextResponse,
  BrowserCurlResponse,
  BrowserElementPickResponse,
  BrowserElementPickResultResponse,
  BrowserRecordAnimationResponse,
  BrowserHandoffResponse,
  BrowserStatusResponse,
} from "../../../services/refact/browser";
import type {
  DiffBox,
  BrowserTabInfo,
  TimelineEntry,
  BrowserFrame,
} from "../browserSlice";

describe("Browser API contract tests", () => {
  test("BrowserStartResponse matches engine struct", () => {
    const response: BrowserStartResponse = {
      runtime_id: "rt-abc",
      status: "started",
    };
    expect(response.runtime_id).toBe("rt-abc");
    expect(response.status).toBe("started");

    const alreadyRunning: BrowserStartResponse = {
      runtime_id: "rt-abc",
      status: "already_running",
    };
    expect(alreadyRunning.status).toBe("already_running");
  });

  test("BrowserStopResponse matches engine struct", () => {
    const response: BrowserStopResponse = {
      status: "stopped",
    };
    expect(response.status).toBe("stopped");
  });

  test("BrowserScreenshotResponse matches engine struct", () => {
    const response: BrowserScreenshotResponse = {
      mime: "image/jpeg",
      data: "base64data",
      url: "https://example.com",
      title: "Example",
    };
    expect(response.mime).toBe("image/jpeg");
    expect(response.data).toBe("base64data");
    expect(response.url).toBe("https://example.com");
    expect(response.title).toBe("Example");
  });

  test("BrowserContextRequest matches engine struct", () => {
    const request: BrowserContextRequest = {
      chat_id: "chat-1",
      max_bytes: 10000,
      last_n_actions: 5,
    };
    expect(request.chat_id).toBe("chat-1");
    expect(request.max_bytes).toBe(10000);
    expect(request.last_n_actions).toBe(5);

    const minimal: BrowserContextRequest = { chat_id: "chat-1" };
    expect(minimal.max_bytes).toBeUndefined();
  });

  test("BrowserContextResponse matches engine struct", () => {
    const response: BrowserContextResponse = {
      url: "https://example.com",
      title: "Example",
      actions: [{ type: "click", selector: "#btn", timestamp: 1000 }],
      console: [{ level: "log", text: "hello", timestamp: 1000 }],
      network: [
        {
          method: "GET",
          url: "https://api.example.com",
          status: 200,
          timestamp: 1000,
        },
      ],
      mutations: [],
      total_bytes: 1234,
    };
    expect(response.url).toBe("https://example.com");
    expect(response.actions).toHaveLength(1);
    expect(response.console).toHaveLength(1);
    expect(response.network).toHaveLength(1);
    expect(response.mutations).toHaveLength(0);
    expect(response.total_bytes).toBe(1234);
  });

  test("BrowserCurlResponse matches engine struct", () => {
    const response: BrowserCurlResponse = {
      curl: "curl 'https://example.com/api'",
      url: "https://example.com/api",
      method: "GET",
      status: 200,
    };
    expect(response.curl).toBe("curl 'https://example.com/api'");
    expect(response.url).toBe("https://example.com/api");
    expect(response.method).toBe("GET");
    expect(response.status).toBe(200);
  });

  test("BrowserElementPickResponse matches engine struct (step 1)", () => {
    const response: BrowserElementPickResponse = {
      status: "picker_active",
    };
    expect(response.status).toBe("picker_active");
  });

  test("BrowserElementPickResultResponse matches engine struct (waiting)", () => {
    const response: BrowserElementPickResultResponse = {
      status: "waiting",
    };
    expect("status" in response).toBe(true);
  });

  test("BrowserElementPickResultResponse matches engine struct (result)", () => {
    const response: BrowserElementPickResultResponse = {
      selector: "#submit-btn",
      innerText: "Submit",
      bbox: { x: 100, y: 200, width: 150, height: 50 },
    };
    if ("selector" in response) {
      expect(response.selector).toBe("#submit-btn");
      expect(response.innerText).toBe("Submit");
      expect(response.bbox.width).toBe(150);
      expect(response.bbox.height).toBe(50);
    }
  });

  test("BrowserRecordAnimationResponse matches engine struct", () => {
    const response: BrowserRecordAnimationResponse = {
      frames: [
        { mime: "image/jpeg", data: "base64frame1", timestamp: 0 },
        { mime: "image/jpeg", data: "base64frame2", timestamp: 200 },
      ],
    };
    expect(response.frames).toHaveLength(2);
    expect(response.frames[0].timestamp).toBe(0);
    expect(response.frames[1].timestamp).toBe(200);
  });

  test("BrowserHandoffResponse matches engine struct", () => {
    const response: BrowserHandoffResponse = {
      runtime_id: "rt-1",
      status: "transferred",
      from_chat_id: "chat-1",
      to_chat_id: "chat-2",
    };
    expect(response.runtime_id).toBe("rt-1");
    expect(response.status).toBe("transferred");
  });

  test("BrowserStatusResponse matches engine struct", () => {
    const connected: BrowserStatusResponse = {
      runtime_id: "rt-1",
      connected: true,
      active_tab: "tab-1",
      url: "https://example.com",
      title: "Example",
      tab_urls: ["https://example.com"],
      tabs: [{ tab_id: "tab-1", url: "https://example.com", title: "Example" }],
      idle_seconds: 30,
      idle_timeout: 300,
    };
    expect(connected.runtime_id).toBe("rt-1");
    expect(connected.connected).toBe(true);
    expect(connected.active_tab).toBe("tab-1");
    expect(connected.tab_urls).toHaveLength(1);
    expect(connected.tabs).toHaveLength(1);

    const disconnected: BrowserStatusResponse = {
      runtime_id: null,
      connected: false,
    };
    expect(disconnected.runtime_id).toBeNull();
    expect(disconnected.connected).toBe(false);
  });

  test("DiffBox uses width/height matching engine struct", () => {
    const box1: DiffBox = { x: 10, y: 20, width: 100, height: 50 };
    expect(box1.width).toBe(100);
    expect(box1.height).toBe(50);
  });

  test("BrowserTabInfo uses tab_id matching engine struct", () => {
    const tab: BrowserTabInfo = {
      tab_id: "tab-1",
      url: "https://example.com",
      title: "Example",
    };
    expect(tab.tab_id).toBe("tab-1");
  });

  test("TimelineEntry matches engine struct", () => {
    const entry: TimelineEntry = {
      timestamp: "2025-01-01T10:00:00Z",
      source: "user",
      type: "click",
      summary: "Clicked button",
      details: { selector: "#btn" },
    };
    expect(entry.timestamp).toBe("2025-01-01T10:00:00Z");
    expect(entry.source).toBe("user");
    expect(entry.type).toBe("click");
    expect(entry.summary).toBe("Clicked button");
  });

  test("BrowserActionRequest matches typed action endpoint", () => {
    const request: BrowserActionRequest = {
      chat_id: "chat-1",
      session: "shared_default",
      target: { type: "active" },
      steps: [
        { action: "navigate", url: "https://example.com" },
        {
          action: "fill",
          locator: { by: "css", value: "input[name=q]" },
          text: "refact browser",
        },
      ],
    };
    expect(request.chat_id).toBe("chat-1");
    expect(request.steps).toHaveLength(2);
    expect(request.target?.type).toBe("active");
  });

  test("BrowserActionResponse matches typed action endpoint", () => {
    const response: BrowserActionResponse = {
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
          summary: "Filled <input> with 14 chars",
          field_kind: "text_input",
          fill_strategy: "dom_value_setter",
          verified: true,
          retries: 1,
        },
      ],
      url: "https://example.com",
      title: "Example",
    };
    expect(response.ok).toBe(true);
    expect(response.steps[1].fill_strategy).toBe("dom_value_setter");
    expect(response.steps[1].verified).toBe(true);
  });

  test("BrowserFrame diff_boxes use width/height", () => {
    const frame: BrowserFrame = {
      mime: "image/jpeg",
      data: "base64",
      diff_boxes: [{ x: 0, y: 0, width: 100, height: 100 }],
    };
    expect(frame.diff_boxes[0].width).toBe(100);
    expect(frame.diff_boxes[0].height).toBe(100);
  });
});
