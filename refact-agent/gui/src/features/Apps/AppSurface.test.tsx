import { http, HttpResponse } from "msw";
import { afterEach, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { server } from "../../utils/mockServer";
import { cleanup, render, screen, waitFor } from "../../utils/test-utils";
import { setUpStore } from "../../app/store";
import { AppSurface } from "./AppSurface";
import { appsApi } from "../../services/refact/apps";
import { isAppRecord } from "../../services/refact/apps";
import { publishAppEvent } from "./appStream";
import { appBindingPending } from "./appsSlice";
import { setThemeMode } from "../Config/configSlice";
import type { ReactNode } from "react";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const hostTokens = vi.hoisted(() => ({
  current: {
    "--rf-bg": "#0c0d0f",
    "--rf-color-fg": "rgba(255, 255, 255, 0.92)",
    "--rf-color-accent": "#7f93d8",
    "--rf-app-font-size": "13px",
  } as Record<string, string>,
}));
type HappyWindow = Window & {
  happyDOM?: { settings: { disableJavaScriptEvaluation: boolean } };
};
const happy = (window as HappyWindow).happyDOM;
if (happy) happy.settings.disableJavaScriptEvaluation = true;
const stores: ReturnType<typeof setUpStore>[] = [];
afterEach(async () => {
  // Unmount and finish unregister requests before the shared MSW teardown.
  cleanup();
  for (const store of stores.splice(0)) {
    await Promise.all(store.dispatch(appsApi.util.getRunningMutationsThunk()));
    store.dispatch(appsApi.util.resetApiState());
  }
});
function surfaceStore() {
  const store = setUpStore();
  stores.push(store);
  return store;
}
const realComputedStyle = window.getComputedStyle.bind(window);
vi.spyOn(window, "getComputedStyle").mockImplementation((element, pseudo) => {
  if (!(element instanceof HTMLElement) || !element.dataset.appSurface)
    return realComputedStyle(element, pseudo);
  const values: Partial<Record<string, string>> = {
    ...hostTokens.current,
    "font-size": hostTokens.current["--rf-app-font-size"],
  };
  return {
    getPropertyValue: (name: string) => values[name] ?? "",
  } as CSSStyleDeclaration;
});
vi.mock("./appDocument", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./appDocument")>();
  return {
    ...actual,
    buildAppDocument: (
      options: Parameters<typeof actual.buildAppDocument>[0],
    ) =>
      actual
        .buildAppDocument(options)
        .replace("<script>", '<script type="application/x-test">'),
  };
});

it("keeps a ready bridge and sequential CAS revisions across document state refetch", async () => {
  let revision = 0;
  const expected: number[] = [];
  server.use(
    http.get("*/v1/apps/board/document", () =>
      HttpResponse.json({
        app_id: "board",
        revision: "published",
        html: "<p>Board</p>",
        sources: [],
        state: { revision, value: { revision } },
      }),
    ),
    http.post("*/v1/apps/commands", async ({ request }) => {
      const command = (await request.json()) as Record<string, unknown>;
      expect(new URL(request.url).searchParams.get("chat_id")).toBe("owner");
      if (command.action === "view_register")
        return HttpResponse.json({
          view_id: "view",
          view_token: "secret",
          generation: "uuid",
          revision: "published",
        });
      if (command.action === "state_get")
        return HttpResponse.json({ revision, value: { card: 7 } });
      if (command.action === "state_set") {
        expected.push(Number(command.expected_revision));
        revision++;
        return HttpResponse.json({ revision, value: command.value });
      }
      return HttpResponse.json({ removed: true });
    }),
  );
  const store = surfaceStore();
  const view = render(
    <AppSurface appId="board" bindingId="binding" chatId="owner" />,
    { store },
  );
  const frame = await screen.findByTitle<HTMLIFrameElement>("board");
  const srcdoc = frame.getAttribute("srcdoc") ?? "";
  const match = /"channel":"([a-z0-9]+)"/.exec(srcdoc);
  if (!match) throw new Error("SDK channel missing from generated document");
  const channel = match[1];
  const target = frame.contentWindow;
  if (!target) throw new Error("iframe window missing");
  const post = vi.spyOn(target, "postMessage");
  const send = async (id: string, method: string, params: unknown) => {
    act(() => {
      window.dispatchEvent(
        new MessageEvent("message", {
          source: target,
          origin: "null",
          data: {
            type: "refact-app",
            v: 1,
            channel,
            generation: 1,
            kind: "request",
            id,
            method,
            params,
          },
        }),
      );
    });
    await waitFor(() =>
      expect(
        post.mock.calls.some((args) => {
          const data: unknown = args[0];
          return isAppRecord(data) && data.id === id && data.ok === true;
        }),
      ).toBe(true),
    );
  };
  await send("c1", "ready", { sdkVersion: "1.0.0" });
  await send("c4", "state.get", {});
  expect(
    post.mock.calls.filter(
      ([data]) => isAppRecord(data) && data.method === "state",
    ),
  ).toHaveLength(0);
  expect(post.mock.calls.map((args) => args[0] as unknown)).toContainEqual(
    expect.objectContaining({
      id: "c4",
      ok: true,
      result: { state: { card: 7 }, revision: 0 },
    }),
  );
  await send("c2", "state.set", { state: { card: 1 } });
  await act(async () => {
    await store
      .dispatch(
        appsApi.endpoints.getAppDocument.initiate(
          { appId: "board", bindingId: "binding", chatId: "owner" },
          { forceRefetch: true },
        ),
      )
      .unwrap();
  });
  expect(frame.getAttribute("srcdoc")).toBe(srcdoc);
  await send("c3", "state.set", { state: { card: 2 } });
  await send("c5", "state.get", {});
  expect(
    post.mock.calls
      .map(([data]) => data as unknown)
      .filter((data) => isAppRecord(data) && data.method === "state"),
  ).toEqual([
    expect.objectContaining({ params: { state: { card: 1 }, revision: 1 } }),
    expect.objectContaining({ params: { state: { card: 2 }, revision: 2 } }),
  ]);
  expect(expected).toEqual([0, 1]);
  const source = {
    app_id: "board",
    source_id: "metrics",
    channel: "cpu",
    generation: "source-generation",
    seq: 2,
    status: "data",
    data: { load: 42 },
  };
  act(() => {
    publishAppEvent({
      generation: "stream",
      seq: 1,
      kind: "source",
      hostChatId: "other",
      data: source,
    });
    publishAppEvent({
      generation: "stream",
      seq: 2,
      kind: "source",
      hostChatId: "owner",
      data: source,
    });
    publishAppEvent({
      generation: "stream",
      seq: 3,
      kind: "source",
      hostChatId: "owner",
      data: { ...source, seq: 1, data: { load: 0 } },
    });
  });
  const sourceNotifications = post.mock.calls
    .map((args) => args[0] as unknown)
    .filter((value) => isAppRecord(value) && value.method === "data");
  expect(sourceNotifications).toHaveLength(1);
  expect(sourceNotifications[0]).toMatchObject({
    params: { name: "cpu", payload: { data: { load: 42 } } },
  });
  view.unmount();
  post.mockRestore();
});

function mockApp(onCommand?: (command: Record<string, unknown>) => void) {
  let documents = 0;
  server.use(
    http.get("*/v1/apps/board/document", () => {
      documents++;
      return HttpResponse.json({
        app_id: "board",
        revision: "published",
        html: "<style>:root{--rf-color-accent:red}</style><p>Board</p>",
        sources: [],
        state: { revision: 0, value: { count: 3 } },
      });
    }),
    http.post("*/v1/apps/commands", async ({ request }) => {
      const command = (await request.json()) as Record<string, unknown>;
      onCommand?.(command);
      if (command.action === "view_register")
        return HttpResponse.json({
          view_id: "view",
          view_token: "secret",
          generation: "uuid",
          revision: "published",
        });
      return HttpResponse.json([]);
    }),
  );
  return { documents: () => documents };
}

function TokenHost({ children }: { children: ReactNode }) {
  return <div data-appearance="dark">{children}</div>;
}

async function readySurface() {
  const frame = await screen.findByTitle<HTMLIFrameElement>("board");
  const channel = /"channel":"([a-z0-9]+)"/.exec(frame.srcdoc)?.[1];
  const target = frame.contentWindow;
  if (!channel || !target) throw new Error("frame not ready");
  const post = vi.spyOn(target, "postMessage");
  const send = (fields: Record<string, unknown>) => {
    act(() => {
      window.dispatchEvent(
        new MessageEvent("message", {
          source: target,
          origin: "null",
          data: {
            type: "refact-app",
            v: 1,
            channel,
            generation: 1,
            kind: "request",
            ...fields,
          },
        }),
      );
    });
  };
  const response = async (id: string) => {
    await waitFor(() =>
      expect(
        post.mock.calls.some(
          ([data]) =>
            isAppRecord(data) && data.id === id && data.kind === "response",
        ),
      ).toBe(true),
    );
    return post.mock.calls
      .map(([data]) => data as unknown)
      .find((data) => isAppRecord(data) && data.id === id);
  };
  send({ id: "c1", method: "ready", params: {} });
  await response("c1");
  return { frame, post, send, response };
}

it("never rolls state or the next CAS back when a delayed GET loses to SET", async () => {
  mockApp();
  let release: (() => void) | undefined;
  const expected: unknown[] = [];
  server.use(
    http.post("*/v1/apps/commands", async ({ request }) => {
      const command = (await request.json()) as Record<string, unknown>;
      if (command.action === "view_register")
        return HttpResponse.json({
          view_id: "view",
          view_token: "secret",
          generation: "uuid",
          revision: "published",
        });
      if (command.action === "state_get") {
        await new Promise<void>((resolve) => {
          release = resolve;
        });
        return HttpResponse.json({ revision: 1, value: { count: 1 } });
      }
      if (command.action === "state_set") {
        expected.push(command.expected_revision);
        return HttpResponse.json({
          revision: expected.length + 1,
          value: command.value,
        });
      }
      return HttpResponse.json([]);
    }),
  );
  render(<AppSurface appId="board" bindingId="binding" chatId="owner" />, {
    store: surfaceStore(),
  });
  const { post, send, response } = await readySurface();
  send({ id: "c2", method: "state.get", params: {} });
  await waitFor(() => expect(release).toBeDefined());
  send({ id: "c3", method: "state.set", params: { state: { count: 2 } } });
  await response("c3");
  release?.();
  await response("c2");
  send({ id: "c4", method: "state.set", params: { state: { count: 3 } } });
  await response("c4");
  expect(expected).toEqual([0, 2]);
  expect(post.mock.calls.map(([data]) => data as unknown)).not.toContainEqual(
    expect.objectContaining({
      method: "state",
      params: expect.objectContaining({ state: { count: 1 } }) as unknown,
    }),
  );
});

it("reports ancestor-hidden retained tabs immediately without losing ready or the iframe", async () => {
  const beats: Record<string, unknown>[] = [];
  mockApp((command) => {
    if (command.action === "view_heartbeat") beats.push(command);
  });
  const view = render(
    <div data-testid="retained">
      <AppSurface appId="board" bindingId="binding" chatId="owner" />
    </div>,
    { store: surfaceStore() },
  );
  const { frame } = await readySurface();
  await waitFor(() =>
    expect(beats.at(-1)).toMatchObject({ ready: true, visible: true }),
  );
  const srcdoc = frame.srcdoc;
  act(() => {
    screen.getByTestId("retained").hidden = true;
  });
  await waitFor(() =>
    expect(beats.at(-1)).toMatchObject({
      ready: true,
      visible: false,
      focused: false,
    }),
  );
  act(() => {
    screen.getByTestId("retained").hidden = false;
  });
  await waitFor(() =>
    expect(beats.at(-1)).toMatchObject({ ready: true, visible: true }),
  );
  expect(screen.getByTitle("board")).toBe(frame);
  expect(frame.srcdoc).toBe(srcdoc);
  view.unmount();
});

it("keeps the same iframe and local draft after capture fails and reports the error", async () => {
  const commands: Record<string, unknown>[] = [];
  mockApp((command) => commands.push(command));
  render(<AppSurface appId="board" bindingId="binding" chatId="owner" />, {
    store: surfaceStore(),
  });
  const { frame, post, send } = await readySurface();
  const draft = frame.contentDocument?.createElement("input");
  if (!draft || !frame.contentDocument)
    throw new Error("frame document missing");
  frame.contentDocument.body.appendChild(draft);
  draft.value = "unsaved draft";
  const srcdoc = frame.srcdoc;
  act(() =>
    publishAppEvent({
      generation: "stream",
      seq: 1,
      hostChatId: "owner",
      kind: "view_request",
      data: {
        view_id: "view",
        generation: "uuid",
        method: "capture",
        request_id: "capture-1",
      },
    }),
  );
  await waitFor(() =>
    expect(post.mock.calls.map(([data]) => data as unknown)).toContainEqual(
      expect.objectContaining({ kind: "request", method: "capture" }),
    ),
  );
  const capture = post.mock.calls
    .map(([data]) => data as unknown)
    .find((data) => isAppRecord(data) && data.method === "capture");
  if (!isAppRecord(capture)) throw new Error("capture request missing");
  send({
    kind: "response",
    id: capture.id,
    ok: false,
    error: { code: "capture_failed", message: "canvas unavailable" },
  });
  await waitFor(() =>
    expect(commands).toContainEqual(
      expect.objectContaining({
        action: "capture_response",
        request_id: "capture-1",
        data_url: null,
        error: "canvas unavailable",
      }),
    ),
  );
  expect(screen.getByRole("status").textContent).toContain("Screenshot failed");
  expect(screen.getByTitle("board")).toBe(frame);
  expect(frame.srcdoc).toBe(srcdoc);
  expect(frame.contentDocument.body.contains(draft)).toBe(true);
  expect(draft.value).toBe("unsaved draft");
});

it("routes focus and close only for the matching view generation and host scope", async () => {
  mockApp();
  const onFocus = vi.fn();
  const onClose = vi.fn();
  render(
    <AppSurface
      appId="board"
      bindingId="binding"
      chatId="owner"
      onFocus={onFocus}
      onClose={onClose}
    />,
    { store: surfaceStore() },
  );
  const { frame } = await readySurface();
  const event = (method: string, generation = "uuid", hostChatId = "owner") =>
    act(() =>
      publishAppEvent({
        generation: "stream",
        seq: 1,
        hostChatId,
        kind: "view_request",
        data: { view_id: "view", generation, method },
      }),
    );
  event("focus", "old");
  event("close", "uuid", "other");
  expect(onFocus).not.toHaveBeenCalled();
  expect(onClose).not.toHaveBeenCalled();
  event("focus");
  expect(onFocus).toHaveBeenCalledOnce();
  expect(window.document.activeElement).toBe(frame);
  event("close");
  expect(onClose).toHaveBeenCalledOnce();
});

it("fills the pane and ships tokens, base styles and the actual appearance initially", async () => {
  mockApp();
  const store = surfaceStore();
  store.dispatch(setThemeMode("dark"));
  render(
    <TokenHost>
      <AppSurface appId="board" bindingId="binding" chatId="owner" />
    </TokenHost>,
    { store },
  );
  const frame = await screen.findByTitle<HTMLIFrameElement>("board");
  const root = frame.closest("[data-app-surface]");
  expect(root?.className).toMatch(/root/);
  expect(frame.className).toMatch(/frame/);
  const srcdoc = frame.getAttribute("srcdoc") ?? "";
  expect(srcdoc).toContain('data-refact-appearance="dark"');
  expect(srcdoc).toContain('<meta name="color-scheme" content="dark">');
  expect(srcdoc).toContain('"appearance":"dark"');
  expect(srcdoc).toMatch(
    /<style id="refact-app-theme-tokens">:root\{[^<]*--rf-bg:#0c0d0f;/,
  );
  expect(srcdoc).toContain("--rf-color-accent:#7f93d8;");
  expect(srcdoc).toContain('<style id="refact-app-theme-base">');
  expect(srcdoc.indexOf("refact-app-theme-tokens")).toBeLessThan(
    srcdoc.indexOf("--rf-color-accent:red"),
  );
});

it("pushes live theme tokens without rebuilding the iframe document", async () => {
  const { documents } = mockApp();
  const store = surfaceStore();
  store.dispatch(setThemeMode("dark"));
  render(
    <TokenHost>
      <AppSurface appId="board" bindingId="binding" chatId="owner" />
    </TokenHost>,
    { store },
  );
  const frame = await screen.findByTitle<HTMLIFrameElement>("board");
  const srcdoc = frame.getAttribute("srcdoc") ?? "";
  const channel = /"channel":"([a-z0-9]+)"/.exec(srcdoc)?.[1];
  const target = frame.contentWindow;
  if (!channel || !target) throw new Error("frame not ready");
  const post = vi.spyOn(target, "postMessage");
  act(() => {
    window.dispatchEvent(
      new MessageEvent("message", {
        source: target,
        origin: "null",
        data: {
          type: "refact-app",
          v: 1,
          channel,
          generation: 1,
          kind: "request",
          id: "c1",
          method: "ready",
          params: { sdkVersion: "1.0.0" },
        },
      }),
    );
  });
  await waitFor(() =>
    expect(post.mock.calls.map((args) => args[0] as unknown)).toContainEqual(
      expect.objectContaining({
        id: "c1",
        ok: true,
        result: expect.objectContaining({
          appearance: "dark",
          tokens: expect.objectContaining({ "--rf-bg": "#0c0d0f" }) as unknown,
        }) as unknown,
      }),
    ),
  );
  const fetched = documents();
  act(() => {
    hostTokens.current = {
      ...hostTokens.current,
      "--rf-bg": "#fcfcfd",
      "--rf-color-fg": "rgba(0, 0, 0, 0.88)",
    };
    store.dispatch(setThemeMode("light"));
  });
  await waitFor(() =>
    expect(post.mock.calls.map((args) => args[0] as unknown)).toContainEqual(
      expect.objectContaining({
        kind: "notification",
        method: "theme",
        params: expect.objectContaining({
          appearance: "light",
          tokens: expect.objectContaining({
            "--rf-bg": "#fcfcfd",
            "--rf-color-fg": "rgba(0, 0, 0, 0.88)",
          }) as unknown,
        }) as unknown,
      }),
    ),
  );
  expect(screen.getByTitle("board")).toBe(frame);
  expect(frame.getAttribute("srcdoc")).toBe(srcdoc);
  expect(documents()).toBe(fetched);
  post.mockRestore();
});

it("waits for a pending binding before fetching or registering the view", async () => {
  const commands: string[] = [];
  const { documents } = mockApp((command) =>
    commands.push(String(command.action)),
  );
  const store = surfaceStore();
  store.dispatch(appBindingPending({ bindingId: "binding", pending: true }));
  render(<AppSurface appId="board" bindingId="binding" chatId="owner" />, {
    store,
  });
  expect(screen.getByRole("status").textContent).toContain("Loading app");
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(documents()).toBe(0);
  expect(commands).not.toContain("view_register");
  act(() => {
    store.dispatch(appBindingPending({ bindingId: "binding", pending: false }));
  });
  await screen.findByTitle("board");
  expect(commands).toContain("view_register");
});

it("contains fill CSS so the iframe never falls back to 300px", () => {
  const css = readFileSync(resolve(__dirname, "AppSurface.module.css"), "utf8");
  const block = (name: string) =>
    new RegExp(`\\.${name} \\{([^}]*)\\}`).exec(css)?.[1] ?? "";
  for (const name of ["root", "surface", "frame"]) {
    expect(block(name)).toContain("flex: 1 1 auto");
    expect(block(name)).toContain("width: 100%");
    expect(block(name)).toContain("min-width: 0");
  }
  expect(block("root")).toContain("align-self: stretch");
  expect(block("frame")).toContain("background: var(--rf-bg)");
});
