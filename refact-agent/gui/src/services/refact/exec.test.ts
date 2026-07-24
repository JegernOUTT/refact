import { afterEach, describe, expect, test, vi } from "vitest";

import type { EngineApiConnection } from "./chatCommands";
import {
  execSubscribeUrl,
  killExec,
  readExec,
  resizeExec,
  writeProcessStdin,
} from "./exec";

const CONNECTION: EngineApiConnection = {
  host: "ide",
  lspPort: 8123,
};

type FetchLike = (
  input: RequestInfo | URL,
  init?: RequestInit,
) => Promise<Response>;

function capturedRequest(
  fetchMock: ReturnType<typeof vi.fn<FetchLike>>,
  index: number,
): Request {
  const [input, init] = fetchMock.mock.calls[index];
  return input instanceof Request ? input : new Request(input, init);
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("exec service chat ownership", () => {
  test("sends chat_id on every process operation without changing auth", async () => {
    const fetchMock = vi.fn<FetchLike>().mockImplementation(() =>
      Promise.resolve(
        new Response("{}", {
          status: 200,
          headers: { "Content-Type": "application/json" },
        }),
      ),
    );
    vi.stubGlobal("fetch", fetchMock);

    await readExec("process/one", 17, CONNECTION, "chat-a", "secret", true);
    await killExec("process/one", CONNECTION, "chat-a", "secret");
    await resizeExec("process/one", 40, 120, CONNECTION, "chat-a", "secret");
    await writeProcessStdin(
      "process/one",
      "hello",
      CONNECTION,
      "chat-a",
      "secret",
    );

    const read = capturedRequest(fetchMock, 0);
    expect(read.url).toBe(
      "http://127.0.0.1:8123/v1/exec/process%2Fone/read?since_seq=17&limit=10000&raw=true&chat_id=chat-a",
    );
    expect(read.headers.get("Authorization")).toBe("Bearer secret");

    const kill = capturedRequest(fetchMock, 1);
    expect(kill.url).toBe(
      "http://127.0.0.1:8123/v1/exec/process%2Fone/kill?chat_id=chat-a",
    );
    expect(kill.method).toBe("POST");
    await expect(kill.clone().json()).resolves.toEqual({});

    const resize = capturedRequest(fetchMock, 2);
    await expect(resize.clone().json()).resolves.toEqual({
      rows: 40,
      cols: 120,
      chat_id: "chat-a",
    });

    const stdin = capturedRequest(fetchMock, 3);
    await expect(stdin.clone().json()).resolves.toEqual({
      chars: "hello",
      chat_id: "chat-a",
    });
  });

  test("includes chat_id and the raw cursor in the EventSource URL", () => {
    expect(execSubscribeUrl("process/one", CONNECTION, "chat-a", 23)).toBe(
      "http://127.0.0.1:8123/v1/exec/process%2Fone/subscribe?since_seq=23&chat_id=chat-a",
    );
  });
});
