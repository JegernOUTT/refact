import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import {
  createDefaultChatState,
  render,
  screen,
  waitFor,
} from "../../utils/test-utils";
import { server } from "../../utils/mockServer";
import { RefreshTools } from "./RefreshTools";
import { updateConfig } from "../../features/Config/configSlice";

describe("explicit tool refresh", () => {
  it("rejects a successful response for another chat", async () => {
    server.use(
      http.post("*/v1/trajectory/refresh-tools", () =>
        HttpResponse.json({
          chat_id: "wrong-chat",
          refreshed: true,
          effective_next_turn: true,
        }),
      ),
    );
    const chat = createDefaultChatState();
    const id = chat.current_thread_id;
    chat.threads[id].snapshot_received = true;
    const { user } = render(<RefreshTools chatId={id} busy={false} />, {
      preloadedState: {
        chat,
        config: { host: "vscode", themeProps: {}, lspPort: 8001 },
      },
    });
    await user.click(
      screen.getByRole("button", { name: "Refresh tools for next turn" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Unable to refresh tools",
    );
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("does not acknowledge an old endpoint response after switching engines", async () => {
    let finish!: () => void;
    const pending = new Promise<void>((resolve) => {
      finish = resolve;
    });
    server.use(
      http.post("*/v1/trajectory/refresh-tools", async ({ request }) => {
        const body = (await request.json()) as { chat_id: string };
        await pending;
        return HttpResponse.json({
          chat_id: body.chat_id,
          refreshed: true,
          effective_next_turn: true,
        });
      }),
    );
    const chat = createDefaultChatState();
    const id = chat.current_thread_id;
    chat.threads[id].snapshot_received = true;
    const { user, store } = render(<RefreshTools chatId={id} busy={false} />, {
      preloadedState: {
        chat,
        config: { host: "vscode", themeProps: {}, lspPort: 8001 },
      },
    });
    await user.click(
      screen.getByRole("button", { name: "Refresh tools for next turn" }),
    );
    expect(
      screen.getByRole("button", { name: "Refreshing tools…" }),
    ).toBeDisabled();
    store.dispatch(updateConfig({ lspPort: 8002 }));
    finish();
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Refresh tools for next turn" }),
      ).toBeEnabled(),
    );
    expect(screen.queryByRole("status")).not.toBeInTheDocument();
  });

  it("only refreshes on click through the current authenticated engine and does not send commands", async () => {
    const requests: unknown[] = [];
    const auth: (string | null)[] = [];
    let commands = 0;
    server.use(
      http.post(
        "http://127.0.0.1:8001/v1/trajectory/refresh-tools",
        async ({ request }) => {
          const body = (await request.json()) as { chat_id: string };
          requests.push(body);
          auth.push(request.headers.get("authorization"));
          return HttpResponse.json({
            chat_id: body.chat_id,
            refreshed: true,
            effective_next_turn: true,
          });
        },
      ),
      http.post("*/v1/chats/:id/commands", () => {
        commands += 1;
        return HttpResponse.json({});
      }),
    );
    const chat = createDefaultChatState();
    const id = chat.current_thread_id;
    chat.threads[id].snapshot_received = true;
    const { user } = render(<RefreshTools chatId={id} busy={false} />, {
      preloadedState: {
        chat,
        config: {
          host: "vscode",
          themeProps: {},
          lspPort: 8001,
          apiKey: "refresh-token",
        },
      },
    });
    expect(requests).toEqual([]);
    await user.click(
      screen.getByRole("button", { name: "Refresh tools for next turn" }),
    );
    expect(await screen.findByRole("status")).toHaveTextContent(
      "Tools refreshed for the next turn. No message was sent.",
    );
    expect(requests).toEqual([{ chat_id: id }]);
    expect(auth).toEqual(["Bearer refresh-token"]);
    expect(commands).toBe(0);
  });

  it.each(["busy", "approval", "waiting", "streaming", "compressing"])(
    "disables refresh while %s",
    (condition) => {
      const chat = createDefaultChatState();
      const id = chat.current_thread_id;
      const runtime = chat.threads[id];
      runtime.snapshot_received = true;
      runtime.confirmation.pause = condition === "approval";
      runtime.waiting_for_response = condition === "waiting";
      runtime.streaming = condition === "streaming";
      runtime.is_compressing = condition === "compressing";
      render(<RefreshTools chatId={id} busy={condition === "busy"} />, {
        preloadedState: { chat },
      });
      expect(
        screen.getByRole("button", { name: "Refresh tools for next turn" }),
      ).toBeDisabled();
    },
  );

  it("shows refusal without a success acknowledgement", async () => {
    server.use(
      http.post("*/v1/trajectory/refresh-tools", () =>
        HttpResponse.json({ detail: "busy" }, { status: 409 }),
      ),
    );
    const chat = createDefaultChatState();
    const id = chat.current_thread_id;
    chat.threads[id].snapshot_received = true;
    const { user } = render(<RefreshTools chatId={id} busy={false} />, {
      preloadedState: {
        chat,
        config: { host: "vscode", themeProps: {}, lspPort: 8001 },
      },
    });
    await user.click(
      screen.getByRole("button", { name: "Refresh tools for next turn" }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Unable to refresh tools",
    );
    await waitFor(() =>
      expect(screen.queryByRole("status")).not.toBeInTheDocument(),
    );
  });
});
