import { act } from "react-dom/test-utils";
import { createChatWithId } from "../Chat/Thread/actions";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { cleanup, render, screen } from "@testing-library/react";
import { Provider } from "react-redux";
import { setUpStore } from "../../app/store";
import { push } from "../Pages/pagesSlice";
import { setCurrentProjectInfo } from "../Chat/currentProject";
import { updateConfig } from "../Config/configSlice";
import { collectTabIds, findLeafByTab } from "../ChatPanes/panesTree";
import { pointerDragController } from "../ChatPanes/pointerDrag";
import {
  hasSplittableTabDrag,
  readTabDragSurfaceKey,
  setTabDragData,
  surfaceKeyFromTabDragPayload,
} from "../ChatPanes/tabDrag";
import { openTab, closePaneTab, setPaneActive } from "./workspaceSlice";
import { makeGitDiffSurfaceKey } from "./surfaceKey";
import {
  WorkspaceSurfaceRoutes,
  useWorkspaceSurfaceRouteContext,
} from "./WorkspaceSurfaceRoutes";

const diff = makeGitDiffSurfaceKey({
  root: "/repo",
  path: "a.ts",
  old_path: null,
  left: { rev: "HEAD", path: "a.ts" },
  right: { rev: "WORKTREE", path: "a.ts" },
  kind: "working",
});
const keys = [
  "chat:c",
  "file:/a.ts",
  "task:t",
  "buddy:home",
  "design:d",
  diff,
  "git:main",
];
function Probe() {
  const task = useWorkspaceSurfaceRouteContext("task:t");
  const buddy = useWorkspaceSurfaceRouteContext("buddy:home");
  return (
    <output data-testid="context">{JSON.stringify({ task, buddy })}</output>
  );
}
function surfaces(store: ReturnType<typeof setUpStore>) {
  return Object.values(store.getState().workspace.groups).flatMap((g) =>
    g ? collectTabIds(g.root) : [],
  );
}
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
});
afterEach(() => {
  vi.unstubAllGlobals();
  cleanup();
  pointerDragController.cancel();
  localStorage.clear();
});
describe("all seven real workspace surfaces", () => {
  it("captures Task card and Buddy deep-link metadata across chat route and pane focus", () => {
    const store = setUpStore();
    store.dispatch(
      setCurrentProjectInfo({ name: "repo", workspaceRoots: ["/repo"] }),
    );
    render(
      <Provider store={store}>
        <WorkspaceSurfaceRoutes>
          <Probe />
        </WorkspaceSurfaceRoutes>
      </Provider>,
    );
    act(() => {
      store.dispatch(
        push({ name: "task workspace", taskId: "t", cardId: "card" }),
      );
    });
    expect(store.getState().pages.slice(-1)[0]?.name).toBe("chat");
    expect(surfaces(store)).toContain("task:t");
    act(() => {
      store.dispatch(
        push({
          name: "buddy",
          draftId: "draft",
          workId: "work",
          workProjectId: "project",
        }),
      );
    });
    expect(surfaces(store)).toEqual(
      expect.arrayContaining(["task:t", "buddy:home"]),
    );
    expect(screen.getByTestId("context").textContent).toContain(
      '"cardId":"card"',
    );
    expect(screen.getByTestId("context").textContent).toContain(
      '"workProjectId":"project"',
    );
    const [tabId, group] = Object.entries(store.getState().workspace.groups)[0];
    const leaf = group && findLeafByTab(group.root, "task:t");
    if (!leaf) throw new Error("missing task leaf");
    act(() => {
      store.dispatch(
        setPaneActive({ tabId, leafId: leaf.id, surfaceKey: "task:t" }),
      );
    });
    expect(screen.getByTestId("context").textContent).toContain(
      '"workId":"work"',
    );
    act(() => {
      store.dispatch(
        closePaneTab({ tabId, leafId: leaf.id, surfaceKey: "task:t" }),
      );
    });
    expect(screen.getByTestId("context").textContent).not.toContain('"cardId"');
    expect(surfaces(store)).not.toContain("task:t");
    act(() => {
      store.dispatch(
        setCurrentProjectInfo({ name: "other", workspaceRoots: ["/other"] }),
      );
    });
    expect(screen.getByTestId("context").textContent).toBe("{}");
  });
  it.each(["project", "roots", "endpoint"])(
    "retains startup deep links on initial project arrival, then clears on %s change",
    (boundary) => {
      const store = setUpStore();
      const task = {
        name: "task workspace" as const,
        taskId: "t",
        cardId: "card",
      };
      const buddy = {
        name: "buddy" as const,
        draftId: "draft",
        workId: "work",
        workProjectId: "project",
      };
      store.dispatch(push(task));
      render(
        <Provider store={store}>
          <WorkspaceSurfaceRoutes>
            <Probe />
          </WorkspaceSurfaceRoutes>
        </Provider>,
      );
      expect(store.getState().pages.at(-1)?.name).toBe("chat");
      act(() => {
        store.dispatch(push(buddy));
      });
      const expected = JSON.stringify({ task, buddy });
      expect(screen.getByTestId("context").textContent).toBe(expected);
      act(() => {
        store.dispatch(
          setCurrentProjectInfo({
            name: "repo",
            projectRoot: "/repo",
            workspaceRoots: ["/repo"],
          }),
        );
      });
      expect(store.getState().pages.at(-1)?.name).toBe("chat");
      expect(screen.getByTestId("context").textContent).toBe(expected);
      expect(surfaces(store)).toEqual(
        expect.arrayContaining(["task:t", "buddy:home"]),
      );
      act(() => {
        if (boundary === "endpoint") {
          store.dispatch(updateConfig({ lspUrl: "http://other-engine:9000" }));
        } else {
          store.dispatch(
            setCurrentProjectInfo({
              name: boundary === "project" ? "other" : "repo",
              projectRoot: boundary === "project" ? "/other" : "/repo",
              workspaceRoots:
                boundary === "project" ? ["/other"] : ["/repo", "/extra"],
            }),
          );
        }
      });
      expect(screen.getByTestId("context").textContent).toBe("{}");
    },
  );
  it("clears unbound startup context when the engine changes before project arrival", () => {
    const store = setUpStore();
    store.dispatch(push({ name: "buddy", draftId: "draft" }));
    render(
      <Provider store={store}>
        <WorkspaceSurfaceRoutes>
          <Probe />
        </WorkspaceSurfaceRoutes>
      </Provider>,
    );
    expect(screen.getByTestId("context").textContent).toContain('"draftId"');
    act(() => {
      store.dispatch(updateConfig({ lspUrl: "http://other-engine:9000" }));
    });
    expect(screen.getByTestId("context").textContent).toBe("{}");
    act(() => {
      store.dispatch(
        setCurrentProjectInfo({ name: "repo", workspaceRoots: ["/repo"] }),
      );
    });
    expect(screen.getByTestId("context").textContent).toBe("{}");
  });
  it.each(keys)(
    "native and pointer transports preserve canonical ownership: %s",
    (key) => {
      const data = new Map<string, string>();
      const transfer = {
        get types() {
          return [...data.keys()];
        },
        setData: (type: string, value: string) => data.set(type, value),
        getData: (type: string) => data.get(type) ?? "",
      } as unknown as DataTransfer;
      setTabDragData(transfer, "surface", key, key);
      expect(readTabDragSurfaceKey(transfer)).toBe(key);
      expect(
        hasSplittableTabDrag({
          types: transfer.types,
          getData: () => "",
        } as unknown as DataTransfer),
      ).toBe(true);
      const store = setUpStore();
      if (key === "chat:c") store.dispatch(createChatWithId({ id: "c" }));
      const unregister = pointerDragController.registerZone({
        id: "all-seven",
        getRect: () => ({
          left: 0,
          top: 0,
          right: 100,
          bottom: 100,
          width: 100,
          height: 100,
        }),
        accepts: (payload) => surfaceKeyFromTabDragPayload(payload) !== null,
        setHover: () => undefined,
        onDrop: (payload) => {
          const target = surfaceKeyFromTabDragPayload(payload);
          if (target) store.dispatch(openTab(target));
        },
      });
      pointerDragController.startDrag(
        { payload: { type: "surface", id: key, surfaceKey: key } },
        { x: 10, y: 10 },
      );
      window.dispatchEvent(
        new PointerEvent("pointerup", { clientX: 10, clientY: 10 }),
      );
      unregister();
      expect(surfaces(store)).toContain(key);
    },
  );
  it("rejects foreign text and invalid Buddy/diff keys", () => {
    expect(
      hasSplittableTabDrag({
        types: ["text/plain"],
        getData: () => "task:t",
      } as unknown as DataTransfer),
    ).toBe(false);
    for (const key of ["buddy:foreign", "gitdiff:broken", "dashboard"])
      expect(
        surfaceKeyFromTabDragPayload({ type: "surface", id: key }),
      ).toBeNull();
  });
});
