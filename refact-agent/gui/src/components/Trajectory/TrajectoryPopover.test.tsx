import { beforeEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import { render, screen, waitFor, within } from "../../utils/test-utils";
import {
  server,
  goodCaps,
  goodChatModes,
  goodPing,
  goodUser,
} from "../../utils/mockServer";
import { TrajectoryButton } from "./TrajectoryButton";

vi.mock("../Portal/Portal", () => ({
  Portal: ({ children }: { children: JSX.Element }) => children,
}));

const CHAT_ID = "chat-llm-preview";

const engineConfigState = {
  config: { host: "vscode" as const, themeProps: {}, lspPort: 8001 },
};

function usePreviewResponse(body: Record<string, unknown>) {
  const requests: unknown[] = [];
  server.use(
    http.post(
      "*/v1/chats/:id/trajectory/llm-compress/preview",
      async ({ request }) => {
        requests.push(await request.json());
        return HttpResponse.json(body);
      },
    ),
  );
  return requests;
}

async function openAndPreview() {
  const { user } = render(
    <TrajectoryButton chatId={CHAT_ID} initialTab="llm-compress" />,
    { preloadedState: engineConfigState },
  );
  await user.click(screen.getByTestId("trajectory-button"));
  const popover = screen.getByRole("dialog");
  const previewButton = within(popover).getByRole("button", {
    name: "Preview",
  });
  await waitFor(() => expect(previewButton).toBeEnabled());
  await user.click(previewButton);
  return popover;
}

describe("TrajectoryPopover LLM compression preview", () => {
  beforeEach(() => {
    server.use(goodCaps, goodChatModes, goodPing, goodUser);
  });

  it("renders an ineligible preview with null model, window and version", async () => {
    const requests = usePreviewResponse({
      eligible: false,
      trajectory_version: null,
      resolved_model: null,
      context_window: null,
      source_messages: 0,
      approximate_source_tokens: 0,
      reason: "No eligible summary model is configured.",
    });

    const popover = await openAndPreview();

    expect(
      await within(popover).findByText(
        "No eligible summary model is configured.",
      ),
    ).toBeInTheDocument();
    expect(requests).toEqual([{ options: {} }]);
    expect(
      within(popover).getByText("Context rebuild is not currently available"),
    ).toBeInTheDocument();
    expect(
      within(popover).queryByText(/Context window:/),
    ).not.toBeInTheDocument();
    expect(within(popover).queryByText(/Model:/)).not.toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Rebuild context" }),
    ).toBeDisabled();
  });

  it("renders an eligible preview with a numeric context window", async () => {
    usePreviewResponse({
      eligible: true,
      trajectory_version: 3,
      resolved_model: "openai/gpt-4o",
      context_window: 128000,
      source_messages: 4,
      approximate_source_tokens: 1200,
      reason: null,
    });

    const popover = await openAndPreview();

    expect(
      await within(popover).findByText(/4 messages \(~1,200 tokens\) eligible/),
    ).toBeInTheDocument();
    expect(
      within(popover).getByText("Model: openai/gpt-4o"),
    ).toBeInTheDocument();
    expect(within(popover).getByText(/Context window:/)).toHaveTextContent(
      "Context window: 128K",
    );
    expect(
      within(popover).getByRole("button", { name: "Rebuild context" }),
    ).toBeEnabled();
  });

  it("omits the context window row when the field is missing", async () => {
    usePreviewResponse({
      eligible: true,
      trajectory_version: 1,
      resolved_model: "openai/gpt-4o",
      source_messages: 2,
      approximate_source_tokens: 50,
    });

    const popover = await openAndPreview();

    expect(
      await within(popover).findByText(/2 messages \(~50 tokens\) eligible/),
    ).toBeInTheDocument();
    expect(
      within(popover).queryByText(/Context window:/),
    ).not.toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Rebuild context" }),
    ).toBeEnabled();
  });
});
