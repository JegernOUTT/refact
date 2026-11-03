import { beforeEach, describe, expect, it, vi } from "vitest";
import { http, HttpResponse } from "msw";
import {
  cleanup,
  render,
  screen,
  waitFor,
  within,
} from "../../utils/test-utils";
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

  it("shows ineligible reasons and eligible preview details", async () => {
    const requests = usePreviewResponse({
      eligible: false,
      trajectory_version: null,
      resolved_model: null,
      context_window: null,
      source_messages: 0,
      approximate_source_tokens: 0,
      reason: "No eligible summary model is configured.",
    });
    const ineligible = await openAndPreview();
    expect(
      await within(ineligible).findByText(
        "No eligible summary model is configured.",
      ),
    ).toBeInTheDocument();
    expect(requests).toEqual([{ options: {} }]);
    expect(within(ineligible).queryByText("Model")).not.toBeInTheDocument();
    expect(
      within(ineligible).getByRole("button", { name: "Rebuild context" }),
    ).toBeDisabled();
    cleanup();

    usePreviewResponse({
      eligible: true,
      trajectory_version: 3,
      resolved_model: "openai/gpt-4o",
      context_window: 128000,
      source_messages: 4,
      approximate_source_tokens: 1200,
      reason: null,
    });
    const eligible = await openAndPreview();
    expect(
      await within(eligible).findByText(
        /4 messages \(~1,200 tokens\) eligible/,
      ),
    ).toBeInTheDocument();
    expect(within(eligible).getByText("openai/gpt-4o")).toBeInTheDocument();
    expect(within(eligible).getByText("128K")).toBeInTheDocument();
    expect(
      within(eligible).getByRole("button", { name: "Rebuild context" }),
    ).toBeEnabled();
  });
});
