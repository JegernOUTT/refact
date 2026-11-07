import { createServer, type ServerResponse } from "node:http";
import { expect, test } from "@playwright/test";
import type {} from "./chat-stream-multiplex";

test("visible chat streams leave HTTP/1.1 capacity for sends and question replies", async ({
  page,
}) => {
  const streams = new Set<ServerResponse>();
  const chatStreams = new Set<ServerResponse>();
  const commands: { chatId: string; type: string; requestId: string }[] = [];
  const server = createServer((request, response) => {
    response.setHeader("Access-Control-Allow-Origin", "*");
    response.setHeader("Access-Control-Allow-Headers", "content-type");
    response.setHeader("Access-Control-Allow-Methods", "GET, POST, OPTIONS");
    if (request.method === "OPTIONS") {
      response.writeHead(204).end();
      return;
    }
    const url = new URL(request.url ?? "/", "http://localhost");
    if (url.pathname.startsWith("/panel/")) {
      response.writeHead(200, { "Content-Type": "text/event-stream" });
      response.write(": panel ready\n\n");
      streams.add(response);
      response.on("close", () => streams.delete(response));
      return;
    }
    if (
      url.pathname === "/v1/chats/subscribe-many" ||
      url.pathname === "/v1/chats/subscribe"
    ) {
      const ids: unknown = url.pathname.endsWith("subscribe-many")
        ? JSON.parse(url.searchParams.get("chat_ids") ?? "[]")
        : [url.searchParams.get("chat_id")];
      if (!Array.isArray(ids) || !ids.every((id) => typeof id === "string")) {
        response.writeHead(400).end();
        return;
      }
      response.writeHead(200, { "Content-Type": "text/event-stream" });
      streams.add(response);
      chatStreams.add(response);
      response.on("close", () => {
        streams.delete(response);
        chatStreams.delete(response);
      });
      for (const chatId of ids) {
        response.write(
          `data: ${JSON.stringify({
            type: "snapshot",
            chat_id: chatId,
            seq: "1",
            messages: [],
            background_agents: [],
          })}\n\n`,
        );
      }
      return;
    }
    const commandPath = /^\/v1\/chats\/([^/]+)\/commands$/.exec(url.pathname);
    if (request.method === "POST" && commandPath) {
      const chunks: Buffer[] = [];
      request.on("data", (chunk: Buffer) => chunks.push(chunk));
      request.on("end", () => {
        const body = JSON.parse(Buffer.concat(chunks).toString()) as {
          type: string;
          client_request_id: string;
        };
        commands.push({
          chatId: commandPath[1],
          type: body.type,
          requestId: body.client_request_id,
        });
        response.writeHead(202, { "Content-Type": "application/json" });
        response.end('{"status":"accepted"}');
      });
      return;
    }
    response.writeHead(404).end();
  });
  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const address = server.address();
  if (address === null || typeof address === "string") {
    throw new Error("Expected isolated TCP listener");
  }
  const engineUrl = `http://127.0.0.1:${address.port}`;
  try {
    await page.goto("/tests/e2e/chat-stream-multiplex.html");
    await page.waitForFunction(() => Boolean(window.chatStreamHarness));
    await page.evaluate(
      (url) => window.chatStreamHarness.start(url),
      engineUrl,
    );
    await expect
      .poll(() =>
        page.evaluate(() => window.chatStreamHarness.state().ready.length),
      )
      .toBeGreaterThanOrEqual(3);
    await page.evaluate((url) => window.chatStreamHarness.send(url), engineUrl);
    await expect
      .poll(() =>
        [...commands].sort((a, b) => a.chatId.localeCompare(b.chatId)),
      )
      .toEqual([
        {
          chatId: "visible-chat-0",
          type: "user_message",
          requestId: "message-request",
        },
        {
          chatId: "visible-chat-1",
          type: "submit_question_response",
          requestId: "question-request",
        },
      ]);
    await expect
      .poll(() => page.evaluate(() => window.chatStreamHarness.state()))
      .toEqual({
        ready: Array.from({ length: 8 }, (_, index) => `visible-chat-${index}`),
        commandState: "completed",
        errors: [],
      });
    expect(chatStreams.size).toBe(1);
    expect(streams.size).toBe(4);
    await page.evaluate(() => window.chatStreamHarness.stop());
    await expect.poll(() => streams.size).toBe(0);
  } finally {
    await page.evaluate(() => window.chatStreamHarness?.stop()).catch(() => {});
    for (const response of streams) response.destroy();
    server.closeAllConnections();
    await new Promise<void>((resolve) => server.close(() => resolve()));
  }
});
