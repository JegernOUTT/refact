import { describe, expect, it } from "vitest";
import { http, HttpResponse } from "msw";
import { setUpStore } from "../../app/store";
import { server } from "../../utils/mockServer";
import {
  appsApi,
  isAppCatalog,
  isAppDocument,
} from "../../services/refact/apps";
import { appsSlice, appContextStaged, appContextRemoved } from "./appsSlice";
import { isAppWorkspace } from "./appWorkspace";
import { appLayoutToPane } from "./useAppWorkspaceRestore";
import {
  workspaceSlice,
  openTab,
  consolidateWorkspaceTabs,
} from "../Workspace/workspaceSlice";
import { collectTabIds } from "../ChatPanes/panesTree";

describe("app platform boundaries", () => {
  it("omits empty optional request IDs while preserving explicit chat scopes", async () => {
    const requested: string[] = [];
    server.use(
      http.get("*/v1/apps", ({ request }) => {
        requested.push(request.url);
        return HttpResponse.json({ apps: [], diagnostics: [] });
      }),
      http.get("*/v1/apps/board/document", ({ request }) => {
        requested.push(request.url);
        return HttpResponse.json({
          app_id: "board",
          revision: "r1",
          html: "<p>Board</p>",
          sources: [],
          state: { revision: 0, value: {} },
        });
      }),
      http.post("*/v1/apps/commands", ({ request }) => {
        requested.push(request.url);
        return HttpResponse.json({ success: true });
      }),
    );
    const store = setUpStore({
      config: { host: "vscode", lspPort: 9494, themeProps: {} },
    });
    try {
      for (const chatId of [undefined, "", "owner"]) {
        await store.dispatch(appsApi.endpoints.getApps.initiate(chatId));
        await store.dispatch(
          appsApi.endpoints.getAppDocument.initiate({
            appId: "board",
            chatId,
            bindingId: "",
          }),
        );
        await store.dispatch(
          appsApi.endpoints.appCommand.initiate({
            action: "source_list",
            app_id: "board",
            hostChatId: chatId,
          }),
        );
      }
      expect(requested).toEqual(
        ["", "", "?chat_id=owner"].flatMap((query) => [
          `http://127.0.0.1:9494/v1/apps${query}`,
          `http://127.0.0.1:9494/v1/apps/board/document${query}`,
          `http://127.0.0.1:9494/v1/apps/commands${query}`,
        ]),
      );
    } finally {
      store.dispatch(appsApi.util.resetApiState());
    }
  });
  it("accepts backend null-icon catalogs and documents but rejects malformed icons", () => {
    // The backend serializes an absent manifest icon as null, not an omitted key.
    const manifest = {
      schema_version: 1,
      app_id: "board",
      name: "Board",
      description: "Project board",
      icon: null,
      entrypoint: "index.html",
      sources: [],
      actions: [],
      permissions: [],
    };
    const catalogApp = {
      slug: "board",
      manifest,
      revision: "r1",
      active_revision: null,
    };
    const document = {
      app_id: "board",
      revision: "r1",
      html: "<p>Board</p>",
      files: { "index.html": "<p>Board</p>" },
      manifest,
      sources: [],
      state: { revision: 0, value: {} },
    };
    expect(isAppCatalog({ apps: [catalogApp], diagnostics: [] })).toBe(true);
    expect(isAppDocument(document)).toBe(true);

    for (const icon of [undefined, "board.svg", 42, false, {}, []]) {
      const candidateManifest = { ...manifest, icon };
      const valid = icon === undefined || typeof icon === "string";
      expect(
        isAppCatalog({
          apps: [{ ...catalogApp, manifest: candidateManifest }],
          diagnostics: [],
        }),
      ).toBe(valid);
      expect(isAppDocument({ ...document, manifest: candidateManifest })).toBe(
        valid,
      );
    }
  });
  it("rejects malformed catalog and document responses rather than displaying fabricated data", () => {
    expect(
      isAppCatalog({
        apps: [{ slug: "bad", manifest: { schema_version: 1 } }],
        diagnostics: [],
      }),
    ).toBe(false);
    expect(
      isAppDocument({
        app_id: "a",
        revision: "r",
        html: "<p>Hello</p>",
        sources: [],
        state: { revision: -1, value: {} },
      }),
    ).toBe(false);
    expect(
      isAppDocument({
        app_id: "a",
        revision: "r",
        html: "<p>Hello</p>",
        sources: [],
        state: { revision: 0, value: {} },
      }),
    ).toBe(true);
  });
  it("keeps standalone app tabs independent of chat consolidation", () => {
    let state = workspaceSlice.reducer(undefined, openTab("chat:one"));
    state = workspaceSlice.reducer(state, openTab("app:standalone:view"));
    state = workspaceSlice.reducer(state, consolidateWorkspaceTabs());
    expect(state.tabs).toContain("app:standalone:view");
    expect(
      collectTabIds(
        state.groups["chat:one"]?.root ?? {
          kind: "leaf",
          id: "missing",
          tabIds: [],
          activeTabId: null,
        },
      ),
    ).not.toContain("app:standalone:view");
    expect(state.activeTabId).toBe("app:standalone:view");
  });
  it("validates binding references and restores portable self as the owning chat", () => {
    const workspace = {
      schema_version: 1,
      revision: 3,
      bindings: [
        {
          binding_id: "00000000-0000-4000-8000-000000000001",
          app_id: "00000000-0000-4000-8000-000000000002",
          params: {},
          state: {},
        },
      ],
      layout: {
        kind: "leaf",
        tabs: [
          { kind: "chat", ref: "self" },
          { kind: "app", binding_id: "00000000-0000-4000-8000-000000000001" },
        ],
        active: 1,
      },
    };
    expect(isAppWorkspace(workspace)).toBe(true);
    if (!isAppWorkspace(workspace)) throw new Error("valid fixture rejected");
    expect(appLayoutToPane(workspace.layout, "owner")).toMatchObject({
      tabIds: ["chat:owner", "app:00000000-0000-4000-8000-000000000001"],
      activeTabId: "app:00000000-0000-4000-8000-000000000001",
    });
    expect(isAppWorkspace({ ...workspace, bindings: [] })).toBe(false);
  });
  it("accepts valid workspaces above the former 512 KiB JSON cap", () => {
    const blob = "s".repeat(700 * 1024);
    const binding = {
      binding_id: "00000000-0000-4000-8000-000000000001",
      app_id: "00000000-0000-4000-8000-000000000002",
      params: { seed: blob },
      state: { notes: blob, list: [blob] },
    };
    const workspace = {
      schema_version: 1,
      revision: 9,
      bindings: [binding],
      layout: {
        kind: "leaf",
        tabs: [{ kind: "app", binding_id: binding.binding_id }],
      },
    };
    expect(JSON.stringify(workspace).length).toBeGreaterThan(2 * 1024 * 1024);
    expect(isAppWorkspace(workspace)).toBe(true);
    const copy = JSON.parse(JSON.stringify(workspace)) as typeof workspace;
    expect(copy.bindings[0].state.notes).toBe(blob);
    expect(
      isAppWorkspace({
        ...workspace,
        bindings: [{ ...binding, state: blob }],
      }),
    ).toBe(false);
  });
  it("stages context in the fixed bound chat without changing other chat drafts", () => {
    const context = {
      id: "selection",
      app_id: "board",
      binding_id: "00000000-0000-4000-8000-000000000001",
      revision: "r1",
      title: "Card",
      text: "Selected card",
    };
    let state = appsSlice.reducer(
      undefined,
      appContextStaged({ chatId: "owner", context }),
    );
    state = appsSlice.reducer(
      state,
      appContextStaged({
        chatId: "other",
        context: { ...context, id: "other-selection" },
      }),
    );
    state = appsSlice.reducer(
      state,
      appContextRemoved({ chatId: "owner", id: "selection" }),
    );
    expect(state.stagedByChat.owner).toEqual([]);
    expect(state.stagedByChat.other).toEqual([
      { ...context, id: "other-selection" },
    ]);
  });
});
