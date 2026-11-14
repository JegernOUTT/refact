import { createServer, type ServerResponse } from "node:http";
import { setImmediate } from "node:timers/promises";
import type { ChatCommand } from "../../services/refact/chatCommands";

export async function startChatSubscriptionFixture() {
  const streams = new Map<string, Set<ServerResponse>>();
  const requests: { chatId: string; command: ChatCommand }[] = [];
  const subscriptions: URL[] = [];
  const sequences = new Map<string, number>();
  const params = new Map<string, Record<string, unknown>>();
  const tasks = new Set<Promise<void>>();
  const failures: unknown[] = [];

  const envelope = (chatId: string, event: Record<string, unknown>) => ({
    chat_id: chatId,
    seq: String(
      sequences.set(chatId, (sequences.get(chatId) ?? 0) + 1).get(chatId),
    ),
    ...event,
  });
  const emit = (chatId: string, event: Record<string, unknown>) => {
    const value = envelope(chatId, event);
    for (const stream of streams.get(chatId) ?? []) {
      stream.write(`data: ${JSON.stringify(value)}\n\n`);
    }
    return value;
  };
  const snapshot = (chatId: string) => ({
    type: "snapshot",
    thread: {
      id: chatId,
      model: "fixture-model",
      mode: "explore",
      ...params.get(chatId),
    },
    runtime: {
      state: "idle",
      paused: false,
      error: null,
      queue_size: 0,
      pause_reasons: [],
      queued_items: [],
    },
    messages: [],
    background_agents: [],
  });

  const server = createServer((req, res) => {
    const task = (async () => {
      res.setHeader("Access-Control-Allow-Origin", "*");
      res.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
      res.setHeader(
        "Access-Control-Allow-Headers",
        "Content-Type, Authorization",
      );
      if (req.method === "OPTIONS") {
        res.writeHead(204).end();
        return;
      }
      const url = new URL(req.url ?? "/", "http://127.0.0.1");
      if (req.method === "GET" && url.pathname === "/v1/chats/subscribe") {
        const chatId = url.searchParams.get("chat_id");
        if (!chatId || url.searchParams.get("snapshot_chunks") !== "1") {
          res.writeHead(400).end();
          return;
        }
        subscriptions.push(url);
        res.writeHead(200, {
          "Content-Type": "text/event-stream",
          "Cache-Control": "no-cache",
        });
        res.flushHeaders();
        const group = streams.get(chatId) ?? new Set<ServerResponse>();
        streams.set(chatId, group);
        group.add(res);
        res.on("close", () => group.delete(res));
        emit(chatId, snapshot(chatId));
        return;
      }
      const match = /^\/v1\/chats\/([^/]+)\/commands$/.exec(url.pathname);
      if (req.method !== "POST" || !match) {
        res.writeHead(404).end();
        return;
      }
      const chatId = decodeURIComponent(match[1]);
      let body = "";
      for await (const chunk of req) body += String(chunk);
      const command = JSON.parse(body) as ChatCommand;
      requests.push({ chatId, command });
      if (chatId === "reject") {
        res.writeHead(409).end("fixture rejection");
        return;
      }
      const status = command.type === "set_params" ? "applied" : "queued";
      if (command.type === "set_params") {
        params.set(chatId, { ...params.get(chatId), ...command.patch });
        emit(chatId, { type: "thread_updated", ...command.patch });
      }
      emit(chatId, {
        type: "ack",
        client_request_id: command.client_request_id,
        accepted: true,
        result: { [status]: true },
      });
      if (command.type === "user_message") {
        emit(chatId, {
          type: "message_added",
          index: 0,
          message: {
            role: "user",
            message_id: "user-1",
            content: command.content,
          },
        });
        emit(chatId, { type: "stream_started", message_id: "assistant-1" });
      }
      if (command.type === "abort")
        emit(chatId, { type: "message_removed", message_id: "assistant-1" });
      res
        .writeHead(200, { "Content-Type": "application/json" })
        .end(JSON.stringify({ status }));
    })().catch((error: unknown) => {
      failures.push(error);
      res.destroy();
    });
    tasks.add(task);
    void task.finally(() => tasks.delete(task));
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("Missing fixture port");

  return {
    port: address.port,
    config: { host: "ide" as const, lspPort: address.port },
    requests,
    subscriptions,
    failures,
    emit,
    snapshot,
    active: (chatId: string) => streams.get(chatId)?.size ?? 0,
    end: (chatId: string) => {
      for (const stream of streams.get(chatId) ?? []) stream.end();
    },
    async wire(chatId: string, text: string) {
      for (const byte of Buffer.from(text)) {
        for (const stream of streams.get(chatId) ?? [])
          stream.write(Buffer.from([byte]));
        await setImmediate();
      }
    },
    async close() {
      for (const group of streams.values())
        for (const stream of group) stream.destroy();
      server.closeAllConnections();
      await new Promise<void>((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve())),
      );
      await Promise.all(tasks);
      streams.clear();
    },
  };
}
