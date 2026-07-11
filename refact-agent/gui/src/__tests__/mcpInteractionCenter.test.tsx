import { beforeEach, describe, expect, test, vi } from "vitest";
import { http, HttpResponse } from "msw";

import { MCPInteractionCenter } from "../components/MCPInteractionCenter";
import type { MCPInteraction } from "../services/refact/mcpInteractions";
import { server } from "../utils/mockServer";
import { render, screen, waitFor } from "../utils/test-utils";

const PRELOADED_STATE = {
  config: {
    apiKey: "test",
    lspPort: 8001,
    themeProps: {},
    host: "vscode" as const,
  },
};

const NOW = Date.now();

function interaction(overrides: Partial<MCPInteraction>): MCPInteraction {
  return {
    id: "interaction-1",
    config_path: "/tmp/mcp.yaml",
    server_name: "Test MCP",
    kind: "elicitation",
    created_at_ms: NOW,
    timeout_at_ms: NOW + 60_000,
    ...overrides,
  } as MCPInteraction;
}

function mockInteractions(interactions: MCPInteraction[]) {
  server.use(
    http.get("*/v1/mcp/interactions", () => {
      return HttpResponse.json({ interactions });
    }),
  );
}

function mockRespond(onBody: (body: unknown) => void = () => undefined) {
  server.use(
    http.post("*/v1/mcp/interactions/respond", async ({ request }) => {
      onBody(await request.json());
      return HttpResponse.json({ success: true });
    }),
  );
}

describe("MCPInteractionCenter", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockRespond();
  });

  test("renders nothing when no interactions", async () => {
    mockInteractions([]);

    render(<MCPInteractionCenter />, {
      preloadedState: PRELOADED_STATE,
    });

    await waitFor(() => {
      expect(
        screen.queryByText("Server needs your input"),
      ).not.toBeInTheDocument();
    });
    expect(screen.queryByText(/Server needs your input/i)).toBeNull();
    expect(screen.queryByText(/Server requests AI sampling/i)).toBeNull();
  });

  test("elicitation with schema submits typed content", async () => {
    let postedBody: unknown;
    mockInteractions([
      interaction({
        message: "Choose deployment options",
        requested_schema: {
          type: "object",
          properties: {
            project: {
              type: "string",
              title: "Project name",
              description: "Name to deploy",
            },
            dryRun: {
              type: "boolean",
              title: "Dry run",
              default: false,
            },
            environment: {
              type: "string",
              title: "Environment",
              enum: ["staging", "prod"],
              default: "staging",
            },
            retries: {
              type: "integer",
              title: "Retries",
              default: 2,
            },
          },
          required: ["project"],
        },
      }),
    ]);
    mockRespond((body) => {
      postedBody = body;
    });

    const { user } = render(<MCPInteractionCenter />, {
      preloadedState: PRELOADED_STATE,
    });

    expect(
      await screen.findByText("Choose deployment options"),
    ).toBeInTheDocument();
    expect(screen.getByLabelText(/Project name/)).toBeInTheDocument();
    expect(screen.getByText("Name to deploy")).toBeInTheDocument();
    expect(screen.getByLabelText("Dry run")).toBeInTheDocument();
    expect(screen.getByLabelText("Environment")).toBeInTheDocument();

    const submit = screen.getByRole("button", { name: "Submit" });
    expect(submit).toBeDisabled();

    await user.type(screen.getByLabelText(/Project name/), "Refact");
    await user.click(screen.getByLabelText("Dry run"));
    expect(submit).toBeEnabled();

    await user.click(submit);

    await waitFor(() => {
      expect(postedBody).toEqual({
        id: "interaction-1",
        action: "accept",
        content: {
          project: "Refact",
          dryRun: true,
          environment: "staging",
          retries: 2,
        },
      });
    });
  });

  test("decline button POSTs decline", async () => {
    let postedBody: unknown;
    mockInteractions([
      interaction({
        message: "Need input",
        requested_schema: {
          properties: {
            answer: { type: "string", title: "Answer" },
          },
        },
      }),
    ]);
    mockRespond((body) => {
      postedBody = body;
    });

    const { user } = render(<MCPInteractionCenter />, {
      preloadedState: PRELOADED_STATE,
    });

    await user.click(await screen.findByRole("button", { name: "Decline" }));

    await waitFor(() => {
      expect(postedBody).toEqual({ id: "interaction-1", action: "decline" });
    });
  });

  test("sampling approval renders preview and allow posts accept", async () => {
    let postedBody: unknown;
    mockInteractions([
      interaction({
        kind: "sampling_approval",
        preview: "User: summarize this repository",
        message_count: 3,
        max_tokens: 2048,
      }),
    ]);
    mockRespond((body) => {
      postedBody = body;
    });

    const { user } = render(<MCPInteractionCenter />, {
      preloadedState: PRELOADED_STATE,
    });

    expect(
      await screen.findByText("Server requests AI sampling"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("User: summarize this repository"),
    ).toBeInTheDocument();
    expect(screen.getByText("Messages: 3")).toBeInTheDocument();
    expect(screen.getByText("Max tokens: 2048")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Deny" })).toBeInTheDocument();

    await user.click(
      screen.getByRole("button", { name: "Allow for this session" }),
    );

    await waitFor(() => {
      expect(postedBody).toEqual({ id: "interaction-1", action: "accept" });
    });
  });

  test("URL elicitation shows Open in browser", async () => {
    const openSpy = vi.spyOn(window, "open").mockImplementation(() => null);
    mockInteractions([
      interaction({
        message: "Authorize the server",
        url: "https://example.com/oauth",
      }),
    ]);

    const { user } = render(<MCPInteractionCenter />, {
      preloadedState: PRELOADED_STATE,
    });

    expect(await screen.findByText("Authorize the server")).toBeInTheDocument();
    expect(screen.getByText("https://example.com/oauth")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "Open in browser" }));

    expect(openSpy).toHaveBeenCalledWith(
      "https://example.com/oauth",
      "_blank",
      "noopener,noreferrer",
    );
    expect(
      screen.getByRole("button", { name: "I've completed it" }),
    ).toBeInTheDocument();

    openSpy.mockRestore();
  });
});
