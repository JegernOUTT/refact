import { http, HttpResponse } from "msw";
import { afterEach, expect, it, vi } from "vitest";
import { act } from "react-dom/test-utils";
import { setUpStore } from "../../app/store";
import { server } from "../../utils/mockServer";
import { cleanup, render, screen, waitFor } from "../../utils/test-utils";
import { appsApi, type CatalogApp } from "../../services/refact/apps";
import { updateConfig } from "../Config/configSlice";
import { AppRail } from "./AppRail";
import * as workspace from "../Workspace/workspaceSlice";
import { trajectoriesApi } from "../../services/refact/trajectories";
import { openRailApp } from "./appsSlice";
import { publishAppEvent } from "./appStream";
import { Dock } from "../Workspace/Dock/Dock";
import {
  hydrateWorkspace,
  setDockOpen,
  setDockSection,
} from "../Workspace/workspaceSlice";
import {
  resolveWorkspaceDockAvailability,
  isDockSectionAvailable,
} from "../Workspace/workspaceAvailability";

vi.mock("./useAppsStream", () => ({ useAppsStream: vi.fn() }));
const stores: ReturnType<typeof setUpStore>[] = [];
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  for (const store of stores.splice(0)) {
    store.dispatch(appsApi.util.resetApiState());
    store.dispatch(trajectoriesApi.util.resetApiState());
  }
});
function app(id: string, name: string, published = true): CatalogApp {
  return {
    slug: id,
    revision: `draft-${id}`,
    active_revision: published ? `published-${id}` : null,
    manifest: {
      schema_version: 1,
      app_id: id,
      name,
      description: `${name} description`,
      icon: "lucide:timer",
      entrypoint: "ui/index.html",
      sources: [],
      actions: [],
      permissions: [],
    },
  };
}
function setup(
  initial = [
    app("timer", "Focus timer"),
    app("board", "Planning board", false),
  ],
) {
  let apps = initial;
  let reads = 0;
  const commands: Record<string, unknown>[] = [];
  const metadataReads: string[] = [];
  server.use(
    http.get("*/v1/trajectories/:id", ({ params }) => {
      metadataReads.push(String(params.id));
      return HttpResponse.json({
        id: params.id,
        title: "Morning planning",
        model: "test",
        mode: "AGENT",
      });
    }),
    http.get("*/v1/apps", () => {
      reads++;
      return HttpResponse.json({ apps, diagnostics: [] });
    }),
    http.post("*/v1/apps/commands", async ({ request }) => {
      const command = (await request.json()) as Record<string, unknown>;
      commands.push(command);
      if (command.action === "details")
        return HttpResponse.json({
          app: { ...initial[0], files: ["app.json", "ui/index.html"] },
          connectors: [
            {
              source: {
                source_id: "clock",
                channel: "ticks",
                driver: { type: "interval", every_ms: 1000 },
                token: "DO-NOT-RENDER",
              },
              runtime: {
                status: "running",
                data: { secret: "DO-NOT-RENDER" },
                view_token: "DO-NOT-RENDER",
              },
            },
          ],
          chats: [
            {
              chat_id: "owner",
              title: "Morning planning",
              binding_ids: ["binding"],
            },
          ],
          coverage: { complete: false, errors: 1 },
        });
      apps = apps.filter((item) => item.manifest.app_id !== command.app_id);
      return HttpResponse.json({ removed: true });
    }),
  );
  const store = setUpStore();
  store.dispatch(
    updateConfig({ host: "web", lspUrl: "http://127.0.0.1:8001" }),
  );
  stores.push(store);
  const dispatch = vi.spyOn(store, "dispatch");
  store.dispatch(setDockSection("apps"));
  store.dispatch(setDockOpen(false));
  const view = render(
    <>
      <AppRail />
      <Dock />
    </>,
    { store },
  );
  return {
    ...view,
    store,
    dispatch,
    commands,
    metadataReads,
    reads: () => reads,
    setApps: (value: CatalogApp[]) => {
      apps = value;
    },
  };
}

it("has a single Apps launcher and named grid; opening dispatches the coordinator intent and keeps the desktop dock", async () => {
  const view = setup();
  await waitFor(() => expect(view.reads()).toBe(1));
  expect(screen.getAllByRole("button")).toHaveLength(1);
  expect(
    screen.queryByRole("button", { name: "Open Focus timer" }),
  ).not.toBeInTheDocument();
  await view.user.click(screen.getByRole("button", { name: "Apps" }));
  expect(screen.getByRole("region", { name: "Apps" })).toBeInTheDocument();
  expect(screen.getByTestId("workspace-dock-section")).toHaveAttribute(
    "data-section",
    "apps",
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(screen.getByRole("list", { name: "Apps" })).toHaveTextContent(
    "Focus timer",
  );
  expect(
    screen.getByRole("heading", { name: "Planning board" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Open Planning board" }),
  ).toHaveAttribute("aria-disabled", "true");
  await view.user.click(
    screen.getByRole("button", { name: "Open Focus timer" }),
  );
  expect(view.dispatch).toHaveBeenCalledWith(
    openRailApp({ appId: "timer", name: "Focus timer" }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(view.store.getState().workspace.dock).toMatchObject({
    open: true,
    section: "apps",
  });
  await view.user.click(screen.getByRole("button", { name: "Apps" }));
  expect(view.store.getState().workspace.dock?.open).toBe(false);
  await waitFor(() =>
    expect(
      screen.queryByRole("region", { name: "Apps" }),
    ).not.toBeInTheDocument(),
  );
});

it("keeps tile actions icon-only, decorative and keyboard accessible with draft guidance", async () => {
  const published = app("timer", "Focus timer");
  published.manifest.description = "";
  published.icon_artwork = {
    view_box: [0, 0, 24, 24],
    nodes: [
      { tag: "circle", attrs: { cx: "12", cy: "12", r: "8" }, children: [] },
    ],
  };
  const view = setup([published, app("board", "Planning board", false)]);
  await view.user.click(screen.getByRole("button", { name: "Apps" }));
  const open = await screen.findByRole("button", { name: "Open Focus timer" });
  const tile = screen.getByRole("listitem", { name: "Focus timer" });
  expect(tile.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
  expect(tile.querySelector("svg circle")).toHaveAttribute("r", "8");
  expect(tile.querySelector("p")).not.toBeInTheDocument();
  expect(
    screen.queryByText("No description provided."),
  ).not.toBeInTheDocument();
  expect(screen.getByLabelText("2 apps")).toHaveTextContent("2");
  for (const label of [
    "Open Focus timer",
    "Details for Focus timer",
    "Remove Focus timer",
    "Refresh apps",
  ]) {
    const button = screen.getByRole("button", { name: label });
    expect(button.textContent).toBe("");
    expect(button.querySelector("svg")).toBeInTheDocument();
    expect(button.querySelector("button")).not.toBeInTheDocument();
  }
  act(() => open.focus());
  expect(await screen.findByRole("tooltip")).toHaveTextContent(
    "Open Focus timer",
  );
  await view.user.keyboard("{Enter}");
  expect(view.dispatch).toHaveBeenCalledWith(
    openRailApp({ appId: "timer", name: "Focus timer" }),
  );
  const draft = screen.getByRole("button", { name: "Open Planning board" });
  expect(draft).toHaveAttribute("aria-disabled", "true");
  act(() => screen.getByRole("button", { name: "Remove Focus timer" }).focus());
  await view.user.tab();
  expect(draft).toHaveFocus();
  const guidance = await screen.findByRole("tooltip");
  expect(guidance).toHaveTextContent("Publish this app before opening it");
  expect(draft).toHaveAttribute("aria-describedby", guidance.id);
  view.dispatch.mockClear();
  await view.user.keyboard("{Enter} ");
  await view.user.click(draft);
  expect(view.dispatch).not.toHaveBeenCalledWith(
    openRailApp({ appId: "board", name: "Planning board" }),
  );
  const before = view.reads();
  await view.user.click(screen.getByRole("button", { name: "Refresh apps" }));
  await waitFor(() => expect(view.reads()).toBeGreaterThan(before));
});

it("keeps the launcher enabled for an empty catalog and handles tool open requests while closed", async () => {
  const view = setup([]);
  await waitFor(() => expect(view.reads()).toBe(1));
  const button = screen.getByRole("button", { name: "Apps" });
  expect(button).toBeEnabled();
  act(() =>
    publishAppEvent({
      generation: "generation",
      seq: 1,
      kind: "view_request",
      data: { method: "open", app_id: "tool-app" },
    }),
  );
  expect(
    view.dispatch.mock.calls.some(([action]) => typeof action === "function"),
  ).toBe(true);
  await view.user.click(button);
  expect(screen.getByText(/No apps yet/)).toBeInTheDocument();
});

it("shows safe details, files, connectors and connected chats without tokens or payloads", async () => {
  const view = setup();
  await view.user.click(screen.getByRole("button", { name: "Apps" }));
  await view.user.click(
    await screen.findByRole("button", { name: "Details for Focus timer" }),
  );
  expect(screen.getByRole("button", { name: "Back to apps" })).toHaveFocus();
  expect(await screen.findByText("app.json")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Back to apps" })).toHaveFocus();
  expect(screen.getByRole("region", { name: "Connectors" })).toHaveTextContent(
    "clock",
  );
  expect(screen.getByRole("region", { name: "Connectors" })).toHaveTextContent(
    "Running",
  );
  expect(
    screen.getByRole("region", { name: "Connected chats" }),
  ).toHaveTextContent("Morning planning");
  expect(screen.getByText("published-timer")).toBeInTheDocument();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await view.user.click(screen.getByRole("button", { name: "Back to apps" }));
  expect(screen.getByRole("list", { name: "Apps" })).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Details for Focus timer" }),
  ).toHaveFocus();
  await view.user.click(
    screen.getByRole("button", { name: "Details for Focus timer" }),
  );
  await screen.findByRole("button", { name: "Morning planning" });
  expect(screen.queryByText(/DO-NOT-RENDER/)).not.toBeInTheDocument();
  expect(view.commands).toContainEqual({ action: "details", app_id: "timer" });
  await view.user.click(
    screen.getByRole("button", { name: "Morning planning" }),
  );
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  await waitFor(() =>
    expect(view.store.getState().chat.current_thread_id).toBe("owner"),
  );
  expect(view.metadataReads).toEqual(["owner"]);
});

it("cancels without mutation and confirms the selected immutable draft revision despite refetch", async () => {
  const view = setup();
  await view.user.click(screen.getByRole("button", { name: "Apps" }));
  await view.user.click(
    await screen.findByRole("button", { name: "Remove Focus timer" }),
  );
  expect(screen.getByRole("region", { name: "Apps" })).toHaveTextContent(
    "chat bindings are preserved",
  );
  expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
  await view.user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(
    screen.getByRole("button", { name: "Remove Focus timer" }),
  ).toHaveFocus();
  expect(view.commands).toHaveLength(0);
  await view.user.click(
    screen.getByRole("button", { name: "Remove Focus timer" }),
  );
  view.setApps([
    { ...app("timer", "Changed name"), revision: "changed-draft" },
  ]);
  await act(async () => {
    await view.store
      .dispatch(
        appsApi.endpoints.getApps.initiate(
          view.store.getState().chat.current_thread_id,
          { forceRefetch: true },
        ),
      )
      .unwrap();
  });
  const before = view.reads();
  await view.user.click(
    screen.getByRole("button", { name: "Confirm removal" }),
  );
  await waitFor(() =>
    expect(view.commands).toContainEqual({
      action: "remove",
      app_id: "timer",
      expected_revision: "draft-timer",
    }),
  );
  await waitFor(() => expect(view.reads()).toBeGreaterThan(before));
  expect(await screen.findByText(/No apps yet/)).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Refresh apps" })).toHaveFocus();
});

it("keeps failed removals honest and retryable without invalidating the catalog", async () => {
  const view = setup();
  let attempts = 0;
  server.use(
    http.post("*/v1/apps/commands", () => {
      attempts++;
      return HttpResponse.json({ error: "revision conflict" }, { status: 409 });
    }),
  );
  await view.user.click(screen.getByRole("button", { name: "Apps" }));
  await view.user.click(
    await screen.findByRole("button", { name: "Remove Focus timer" }),
  );
  const before = view.reads();
  await view.user.click(
    screen.getByRole("button", { name: "Confirm removal" }),
  );
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "could not be confirmed",
  );
  expect(view.reads()).toBe(before);
  await view.user.click(screen.getByRole("button", { name: "Retry removal" }));
  await waitFor(() => expect(attempts).toBe(2));
  await view.user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(
    screen.getByRole("button", { name: "Open Focus timer" }),
  ).toBeEnabled();
});

it("hydrates the Apps section and allows it in IDE without enabling old panels", () => {
  const store = setUpStore();
  stores.push(store);
  store.dispatch(
    hydrateWorkspace({
      tabs: [],
      activeTabId: null,
      groups: {},
      dock: { open: true, width: 420, section: "apps" },
    }),
  );
  expect(store.getState().workspace.dock).toEqual({
    open: true,
    width: 420,
    section: "apps",
  });
  const available = resolveWorkspaceDockAvailability(
    "vscode",
    { filesPanel: true, gitPanel: true, terminalPanel: true },
    false,
  );
  expect(isDockSectionAvailable("apps", available)).toBe(true);
  expect(available).toMatchObject({
    dock: false,
    files: false,
    git: false,
    terminal: false,
    activity: false,
  });
});

it("uses the existing narrow dock Sheet and closes it when opening an app", async () => {
  vi.spyOn(window, "matchMedia").mockImplementation((query) => ({
    matches: query === "(max-width: 767px)",
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }));
  const view = setup();
  await view.user.click(screen.getByRole("button", { name: "Apps" }));
  expect(
    screen.getByRole("dialog", { name: "Workspace dock" }),
  ).toBeInTheDocument();
  expect(screen.getAllByRole("dialog")).toHaveLength(1);
  await view.user.click(
    await screen.findByRole("button", { name: "Open Focus timer" }),
  );
  expect(view.store.getState().workspace.dock?.open).toBe(false);
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
});

it("uses the focused workspace chat instead of the current thread for both catalog readers", async () => {
  vi.spyOn(workspace, "selectFocusedWorkspaceChatId").mockReturnValue(
    "focused-agent",
  );
  const view = setup();
  await waitFor(() => expect(view.reads()).toBe(1));
  expect(view.store.getState().chat.current_thread_id).not.toBe(
    "focused-agent",
  );
  await view.user.click(screen.getByRole("button", { name: "Apps" }));
  await screen.findByRole("button", { name: "Details for Focus timer" });
  expect(
    appsApi.endpoints.getApps.select("focused-agent")(view.store.getState())
      .isSuccess,
  ).toBe(true);
  expect(view.reads()).toBe(1);
});

it("keeps the narrow details panel open while navigation is pending or fails and closes only on success", async () => {
  vi.spyOn(window, "matchMedia").mockImplementation((query) => ({
    matches: query === "(max-width: 767px)",
    media: query,
    onchange: null,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
    addListener: vi.fn(),
    removeListener: vi.fn(),
    dispatchEvent: vi.fn(),
  }));
  const view = setup();
  let finish!: () => void;
  const pending = new Promise<void>((resolve) => {
    finish = resolve;
  });
  server.use(
    http.get("*/v1/trajectories/:id", async () => {
      await pending;
      return HttpResponse.json({ error: "Chat unavailable" }, { status: 404 });
    }),
  );
  await view.user.click(screen.getByRole("button", { name: "Apps" }));
  await view.user.click(
    await screen.findByRole("button", { name: "Details for Focus timer" }),
  );
  await view.user.click(
    await screen.findByRole("button", { name: "Morning planning" }),
  );
  expect(await screen.findByRole("status")).toHaveTextContent(
    "Opening linked chat",
  );
  expect(
    screen.getByRole("button", { name: "Morning planning" }),
  ).toBeDisabled();
  expect(view.store.getState().workspace.dock?.open).toBe(true);
  act(() => {
    finish();
  });
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Could not open link",
  );
  expect(view.store.getState().workspace.dock?.open).toBe(true);
  expect(
    screen.getByRole("button", { name: "Morning planning" }),
  ).toBeEnabled();
  server.use(
    http.get("*/v1/trajectories/:id", ({ params }) =>
      HttpResponse.json({
        id: params.id,
        title: "Morning planning",
        model: "test",
        mode: "AGENT",
      }),
    ),
  );
  await view.user.click(
    screen.getByRole("button", { name: "Morning planning" }),
  );
  await waitFor(() =>
    expect(view.store.getState().workspace.dock?.open).toBe(false),
  );
  expect(view.store.getState().chat.current_thread_id).toBe("owner");
});

it("restores focus to a surviving app after removal", async () => {
  const view = setup();
  await view.user.click(screen.getByRole("button", { name: "Apps" }));
  await view.user.click(
    await screen.findByRole("button", { name: "Remove Focus timer" }),
  );
  expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
  await view.user.click(
    screen.getByRole("button", { name: "Confirm removal" }),
  );
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Details for Planning board" }),
    ).toHaveFocus(),
  );
});
