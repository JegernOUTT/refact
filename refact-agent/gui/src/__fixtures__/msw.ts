import { http, HttpResponse, type HttpHandler } from "msw";
import { EMPTY_CAPS_RESPONSE, STUB_CAPS_RESPONSE } from "./caps";
import { SYSTEM_PROMPTS } from "./prompts";
import { STUB_LINKS_FOR_CHAT_RESPONSE } from "./chat_links_response";
import { TOOLS, CHAT_LINKS_URL } from "../services/refact/consts";
import { STUB_TOOL_RESPONSE } from "./tools_response";
import type { LinksForChatResponse } from "../services/refact/links";
import { ToolConfirmationResponse } from "../services/refact";

export const goodPing: HttpHandler = http.get(
  "http://127.0.0.1:8001/v1/ping",
  () => {
    return HttpResponse.text("pong");
  },
);

export const goodCaps: HttpHandler = http.get(
  "http://127.0.0.1:8001/v1/caps",
  () => {
    return HttpResponse.json(STUB_CAPS_RESPONSE);
  },
);

export const goodCapsWithKnowledgeFeature: HttpHandler = http.get(
  "http://127.0.0.1:8001/v1/caps",
  () => {
    return HttpResponse.json({
      ...STUB_CAPS_RESPONSE,
      metadata: { features: ["knowledge"] },
    });
  },
);

export const emptyCaps: HttpHandler = http.get(
  `http://127.0.0.1:8001/v1/caps`,
  () => {
    return HttpResponse.json(EMPTY_CAPS_RESPONSE);
  },
);

export const noTools: HttpHandler = http.get(
  "http://127.0.0.1:8001/v1/tools",
  () => {
    return HttpResponse.json([]);
  },
);

export const goodPrompts: HttpHandler = http.get(
  "http://127.0.0.1:8001/v1/customization",
  () => {
    return HttpResponse.json({ system_prompts: SYSTEM_PROMPTS });
  },
);

export const noCompletions: HttpHandler = http.post(
  "http://127.0.0.1:8001/v1/at-command-completion",
  () => {
    return HttpResponse.json({
      completions: [],
      replace: [0, 0],
      is_cmd_executable: false,
    });
  },
);

export const noCommandPreview: HttpHandler = http.post(
  "http://127.0.0.1:8001/v1/at-command-preview",
  () => {
    return HttpResponse.json({
      messages: [],
    });
  },
);

export const goodUser: HttpHandler = http.get(
  "http://127.0.0.1:8001/v1/providers",
  () =>
    HttpResponse.json({
      providers: [
        {
          name: "openai",
          base_provider: "openai",
          display_name: "OpenAI",
          enabled: true,
          readonly: false,
          has_credentials: true,
          status: "active",
          model_count: 5,
        },
      ],
    }),
);

export const chatLinks: HttpHandler = http.post(
  `http://127.0.0.1:8001${CHAT_LINKS_URL}`,
  () => {
    return HttpResponse.json(STUB_LINKS_FOR_CHAT_RESPONSE);
  },
);

export const noChatLinks: HttpHandler = http.post(
  `http://127.0.0.1:8001${CHAT_LINKS_URL}`,
  () => {
    const res: LinksForChatResponse = {
      uncommited_changes_warning: "",
      new_chat_suggestion: false,
      links: [],
    };
    return HttpResponse.json(res);
  },
);

export const goodTools: HttpHandler = http.get(
  `http://127.0.0.1:8001${TOOLS}`,
  () => {
    return HttpResponse.json(STUB_TOOL_RESPONSE);
  },
);

export const ToolConfirmation = http.post(
  "http://127.0.0.1:8001/v1/tools-check-if-confirmation-needed",
  () => {
    const response: ToolConfirmationResponse = {
      pause: false,
      pause_reasons: [],
    };

    return HttpResponse.json(response);
  },
);

export const emptyTrajectories: HttpHandler = http.get(
  "http://127.0.0.1:8001/v1/trajectories",
  () => {
    return HttpResponse.json([]);
  },
);

export const trajectoryGet: HttpHandler = http.get(
  "http://127.0.0.1:8001/v1/trajectories/:id",
  () => {
    return HttpResponse.json({ status: "not_found" }, { status: 404 });
  },
);

export const trajectorySave: HttpHandler = http.put(
  "http://127.0.0.1:8001/v1/trajectories/:id",
  () => {
    return HttpResponse.json({ status: "ok" });
  },
);

export const trajectoryDelete: HttpHandler = http.delete(
  "http://127.0.0.1:8001/v1/trajectories/:id",
  () => {
    return HttpResponse.json({ status: "ok" });
  },
);

// Chat Session (Stateless Trajectory UI) handlers
export const chatSessionSubscribe: HttpHandler = http.get(
  "http://127.0.0.1:8001/v1/chats/subscribe",
  () => {
    // Return an SSE stream that immediately closes (no events)
    const encoder = new TextEncoder();
    const stream = new ReadableStream({
      start(controller) {
        // Send a comment to keep connection alive, then close
        controller.enqueue(encoder.encode(": keep-alive\n\n"));
        // Don't close - let the client handle disconnection
      },
    });
    return new HttpResponse(stream, {
      headers: {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      },
    });
  },
);

export const chatSessionCommand: HttpHandler = http.post(
  "http://127.0.0.1:8001/v1/chats/:id/commands",
  () => {
    return HttpResponse.json({ status: "queued" });
  },
);

export const chatSessionAbort: HttpHandler = http.post(
  "http://127.0.0.1:8001/v1/chats/:id/abort",
  () => {
    return HttpResponse.json({ status: "ok" });
  },
);

// Sidebar subscription endpoint (SSE)
export const sidebarSubscribe: HttpHandler = http.get(
  "http://127.0.0.1:8001/v1/sidebar/subscribe",
  () => {
    const encoder = new TextEncoder();
    const stream = new ReadableStream({
      start(controller) {
        // Send initial snapshot with empty data
        const snapshot = JSON.stringify({
          category: "snapshot",
          trajectories: [],
          tasks: [],
        });
        controller.enqueue(encoder.encode(`data: ${snapshot}\n\n`));
      },
    });
    return new HttpResponse(stream, {
      headers: {
        "Content-Type": "text/event-stream",
        "Cache-Control": "no-cache",
        Connection: "keep-alive",
      },
    });
  },
);

// Tasks list endpoint
export const emptyTasks: HttpHandler = http.get(
  "http://127.0.0.1:8001/v1/tasks",
  () => {
    return HttpResponse.json([]);
  },
);
