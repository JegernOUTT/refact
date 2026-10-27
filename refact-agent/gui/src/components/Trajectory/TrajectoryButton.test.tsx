import { describe, it, expect, vi } from "vitest";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { http, HttpResponse } from "msw";
import {
  createDefaultChatState,
  render,
  screen,
  waitFor,
  within,
} from "../../utils/test-utils";
import { goodCaps, goodPing, server } from "../../utils/mockServer";
import { applyChatEvent } from "../../features/Chat/Thread/actions";
import { TrajectoryButton } from "./TrajectoryButton";

vi.mock("../Portal/Portal", () => ({
  Portal: ({ children }: { children: JSX.Element }) => children,
}));

describe("TrajectoryButton", () => {
  it("renders the trajectory button", () => {
    render(<TrajectoryButton />);
    const button = screen.getByTestId("trajectory-button");
    expect(button).toBeInTheDocument();
  });

  it("has correct aria-label", () => {
    render(<TrajectoryButton />);
    const button = screen.getByLabelText("Compress or rebuild context");
    expect(button).toBeInTheDocument();
  });

  it("uses the standard tab strip geometry for the handoff popover", async () => {
    const css = await readFile(
      path.resolve(__dirname, "TrajectoryPopover.module.css"),
      "utf8",
    );
    const tabStrip = css.match(/\.tabStrip \{[^}]+\}/)?.[0] ?? "";

    expect(tabStrip).toContain("max-width: 100%;");
    expect(tabStrip).not.toContain("width: max-content;");
    expect(tabStrip).not.toContain("grid-auto-columns: max-content;");
  });

  it("previews and applies compression while waiting and rejects actions once streaming", async () => {
    const calls: string[] = [];
    server.use(
      http.post("*/v1/chats/:id/trajectory/transform/preview", () => {
        calls.push("preview");
        return HttpResponse.json({
          stats: { before_approx_tokens: 100, after_approx_tokens: 50 },
          actions: ["Truncated tool results"],
        });
      }),
      http.post("*/v1/chats/:id/trajectory/transform/apply", () => {
        calls.push("apply");
        return HttpResponse.json({
          stats: { before_approx_tokens: 100, after_approx_tokens: 50 },
        });
      }),
    );
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    chat.threads[chatId].waiting_for_response = true;
    const { user, store } = render(<TrajectoryButton chatId={chatId} />, {
      preloadedState: { chat },
    });

    await user.click(screen.getByTestId("trajectory-button"));
    const popover = screen.getByRole("dialog");
    const preview = within(popover).getByRole("button", { name: "Preview" });
    expect(preview).toBeEnabled();
    await user.click(preview);
    await waitFor(() =>
      expect(
        within(popover).getByRole("button", { name: "Apply" }),
      ).toBeEnabled(),
    );
    expect(calls).toEqual(["preview"]);

    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "runtime_updated",
        state: "generating",
      }),
    );
    await waitFor(() =>
      expect(
        within(popover).getByRole("button", { name: "Apply" }),
      ).toBeDisabled(),
    );
    const apply = within(popover).getByRole("button", { name: "Apply" });
    expect(
      within(popover).getByRole("button", { name: "Preview" }),
    ).toBeDisabled();
    await user.click(apply);
    expect(calls).toEqual(["preview"]);

    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: "2",
        type: "runtime_updated",
        state: "waiting_user_input",
      }),
    );
    await waitFor(() =>
      expect(
        within(popover).getByRole("button", { name: "Apply" }),
      ).toBeEnabled(),
    );
    await user.click(within(popover).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(calls).toEqual(["preview", "apply"]));
  });

  it("previews and rebuilds LLM compression while waiting and rejects actions once streaming", async () => {
    const calls: string[] = [];
    server.use(
      goodCaps,
      goodPing,
      http.get("*/v1/chat-modes", () =>
        HttpResponse.json({ modes: [], errors: [] }),
      ),
      http.post("*/v1/chats/:id/trajectory/llm-compress/preview", () => {
        calls.push("preview");
        return HttpResponse.json({
          eligible: true,
          trajectory_version: 3,
          source_messages: 2,
          approximate_source_tokens: 120,
        });
      }),
      http.post("*/v1/chats/:id/trajectory/llm-compress/apply", () => {
        calls.push("apply");
        return HttpResponse.json({ applied: true });
      }),
    );
    const chat = createDefaultChatState();
    const chatId = chat.current_thread_id;
    chat.threads[chatId].waiting_for_response = true;
    const { user, store } = render(
      <TrajectoryButton chatId={chatId} initialTab="llm-compress" />,
      {
        preloadedState: {
          chat,
          config: { host: "vscode", themeProps: {}, lspPort: 8001 },
        },
      },
    );

    await user.click(screen.getByTestId("trajectory-button"));
    const popover = screen.getByRole("dialog");
    const preview = within(popover).getByRole("button", { name: "Preview" });
    await waitFor(() => expect(preview).toBeEnabled());
    await user.click(preview);
    await waitFor(() =>
      expect(
        within(popover).getByRole("button", { name: "Rebuild context" }),
      ).toBeEnabled(),
    );
    expect(within(popover).getByText(/2 messages/)).toBeInTheDocument();
    expect(calls).toEqual(["preview"]);

    store.dispatch(
      applyChatEvent({
        chat_id: chatId,
        seq: "1",
        type: "runtime_updated",
        state: "generating",
      }),
    );
    await waitFor(() =>
      expect(
        within(popover).getByRole("button", { name: "Rebuild context" }),
      ).toBeDisabled(),
    );
    // The already-open preview stays visible but cannot be applied.
    expect(within(popover).getByText(/2 messages/)).toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Preview" }),
    ).toBeDisabled();
    await user.click(
      within(popover).getByRole("button", { name: "Rebuild context" }),
    );
    await user.click(within(popover).getByRole("button", { name: "Preview" }));
    expect(calls).toEqual(["preview"]);
  });

  it("retains static and LLM compression tabs without Handoff with preview-gated actions", async () => {
    const { user } = render(<TrajectoryButton />);

    await user.click(screen.getByTestId("trajectory-button"));

    const popover = screen.getByRole("dialog");
    expect(
      within(popover).getByRole("tab", { name: "Compress in-place" }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("tab", { name: "LLM compression" }),
    ).toBeInTheDocument();

    expect(
      within(popover).queryByRole("tab", { name: "Handoff" }),
    ).not.toBeInTheDocument();
    expect(
      within(popover).getByRole("checkbox", { name: "Drop all context files" }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("checkbox", {
        name: "Remove usage and metering details",
      }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Preview" }),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Apply" }),
    ).toBeInTheDocument();

    await user.click(
      within(popover).getByRole("tab", { name: "LLM compression" }),
    );
    expect(
      within(popover).getByText(/Rebuilds the active conversation context/i),
    ).toBeInTheDocument();
    expect(
      within(popover).getByRole("button", { name: "Rebuild context" }),
    ).toBeDisabled();
  });
});
