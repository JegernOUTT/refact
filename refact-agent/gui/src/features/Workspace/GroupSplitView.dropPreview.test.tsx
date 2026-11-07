import { act } from "react-dom/test-utils";
import { createChatWithId } from "../Chat/Thread";
import { afterEach, describe, expect, it, vi } from "vitest";
import { setUpStore } from "../../app/store";
import { fireEvent, render, screen } from "../../utils/test-utils";
import { pointerDragController } from "../ChatPanes/pointerDrag";
import { collectLeafIds } from "../ChatPanes/panesTree";
import { setTabDragData } from "../ChatPanes/tabDrag";
import { GroupSplitView } from "./GroupSplitView";
import { resolvePaneDropTarget } from "./paneDropTarget";
import {
  addSurfaceToPane,
  openTab,
  setActiveTab,
  splitTab,
} from "./workspaceSlice";

vi.hoisted(() => {
  vi.resetModules();
});

vi.mock("./SurfacePane", () => ({ SurfacePane: () => <div>Surface</div> }));
vi.mock("../Dashboard/hooks/useDashboardLayout", () => ({
  useDashboardLayout: () => "wide",
}));

const bounds = { left: 0, top: 0, width: 400, height: 300 };
function setup(host: "web" | "jetbrains" = "web") {
  const initialStore = setUpStore({
    config: { host, lspPort: 8001, apiKey: "test", themeProps: {} },
  });
  initialStore.dispatch(createChatWithId({ id: "a", title: "A" }));
  initialStore.dispatch(createChatWithId({ id: "b", title: "B" }));
  initialStore.dispatch(openTab("chat:a"));
  initialStore.dispatch(openTab("chat:b"));
  initialStore.dispatch(setActiveTab("chat:a"));
  const initial = initialStore.getState();
  const store = setUpStore({
    ...initial,
    workspace: {
      ...initial.workspace,
      tabs: ["chat:a", "chat:b"],
      activeTabId: "chat:a",
      groups: {
        "chat:a": {
          root: {
            kind: "leaf",
            id: "root",
            tabIds: ["chat:a"],
            activeTabId: "chat:a",
          },
          focusedLeafId: "root",
        },
      },
    },
  });

  render(
    <>
      <div role="tablist" aria-label="Workspace tabs">
        <button data-pane-tab-key="chat:b">Drag B</button>
      </div>
      <GroupSplitView tabId="chat:a" />
    </>,
    { store },
  );
  const pane = screen.getByLabelText("Workspace pane root");
  const controls = pane.querySelector<HTMLElement>("[data-pane-controls]");
  if (controls) controls.style.opacity = "0";
  pane.getBoundingClientRect = () =>
    ({ ...bounds, right: 400, bottom: 300 }) as DOMRect;
  return { store, pane };
}
function transfer(key = "chat:b", typed = true): DataTransfer {
  const data = new Map<string, string>();
  const result = {
    get types() {
      return [...data.keys()];
    },
    setData: (type: string, value: string) => data.set(type, value),
    getData: (type: string) => data.get(type) ?? "",
    dropEffect: "none",
  } as unknown as DataTransfer;
  if (typed) setTabDragData(result, "surface", key, key);
  else result.setData("text/plain", key);
  return result;
}
function native(
  pane: HTMLElement,
  x: number,
  y: number,
  dataTransfer = transfer(),
) {
  const event = new MouseEvent("dragover", {
    clientX: x,
    clientY: y,
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(event, "dataTransfer", { value: dataTransfer });
  fireEvent(pane, event);
}
function pointer(type: "pointermove" | "pointerup", x: number, y: number) {
  act(() => {
    window.dispatchEvent(new MouseEvent(type, { clientX: x, clientY: y }));
  });
}
const preview = () => screen.queryByTestId("workspace-pane-drop-preview");
afterEach(() => {
  act(() => pointerDragController.cancel());
  document.body.removeAttribute("data-rf-native-tab-drag");
});

describe("pane destination geometry", () => {
  it.each([
    [0, 0, "left"],
    [400, 0, "right"],
    [0, 300, "left"],
    [400, 300, "right"],
    [200, 10, "top"],
    [200, 290, "bottom"],
    [200, 150, "center"],
    [-1, 100, null],
  ])("resolves %s,%s to %s deterministically", (x, y, target) => {
    expect(resolvePaneDropTarget(bounds, { x: Number(x), y: Number(y) })).toBe(
      target,
    );
  });
});

describe("single native pane preview", () => {
  it("can split the active local tab out of a multi-tab pane but not join it to itself", async () => {
    const { store } = setup();
    await act(async () => {
      store.dispatch(
        addSurfaceToPane({
          tabId: "chat:a",
          leafId: "root",
          surfaceKey: "chat:b",
        }),
      );
      await Promise.resolve();
    });
    const pane = screen.getByLabelText("Workspace pane root");
    pane.getBoundingClientRect = () =>
      ({ ...bounds, right: 400, bottom: 300 }) as DOMRect;
    native(pane, 200, 150);
    expect(preview()).not.toBeInTheDocument();
    native(pane, 390, 150);
    expect(preview()).toHaveAttribute("data-drop-target", "right");
    const event = new MouseEvent("drop", {
      clientX: 390,
      clientY: 150,
      bubbles: true,
    });
    Object.defineProperty(event, "dataTransfer", { value: transfer() });
    fireEvent(pane, event);
    const group = store.getState().workspace.groups["chat:a"];
    if (!group) throw new Error("Missing split group");
    expect(collectLeafIds(group.root)).toHaveLength(2);
  });

  it("neutralizes iframe input only during an internal native drag and restores it on cancel", () => {
    setup();
    fireEvent.dragStart(screen.getByText("Drag B"), {
      dataTransfer: transfer(),
    });
    expect(document.body).toHaveAttribute("data-rf-native-tab-drag", "true");
    fireEvent.keyDown(window, { key: "Escape" });
    expect(document.body).not.toHaveAttribute("data-rf-native-tab-drag");
    fireEvent.dragStart(screen.getByText("Drag B"), {
      dataTransfer: transfer(),
    });
    fireEvent.dragEnd(window);
    expect(document.body).not.toHaveAttribute("data-rf-native-tab-drag");
  });

  it("does not offer a split beyond the pane limit while keeping center joining available", async () => {
    const { store } = setup();
    for (let index = 0; index < 5; index++) {
      await act(async () => {
        store.dispatch(
          splitTab({
            tabId: "chat:a",
            dir: "row",
            surfaceKey: `file:/pane-${index}.txt`,
          }),
        );
        await Promise.resolve();
      });
    }
    const group = store.getState().workspace.groups["chat:a"];
    if (!group) throw new Error("Missing six-pane group");
    expect(collectLeafIds(group.root)).toHaveLength(6);
    const first = screen.getByLabelText("Workspace pane root");
    first.getBoundingClientRect = () =>
      ({ ...bounds, right: 400, bottom: 300 }) as DOMRect;
    const controls = first.querySelector<HTMLElement>("[data-pane-controls]");
    if (controls) controls.style.opacity = "0";
    const before = store.getState().workspace;
    native(first, 10, 150);
    expect(preview()).not.toBeInTheDocument();
    native(first, 200, 150);
    expect(preview()).toHaveAttribute("data-drop-target", "center");
    expect(store.getState().workspace).toBe(before);
  });

  it("switches one preview across edges and center without changing workspace before drop", () => {
    const { pane, store } = setup();
    const before = store.getState().workspace;
    for (const [x, y, target] of [
      [10, 150, "left"],
      [390, 150, "right"],
      [200, 10, "top"],
      [200, 290, "bottom"],
      [200, 150, "center"],
    ] as const) {
      native(pane, x, y);
      expect(screen.getAllByTestId("workspace-pane-drop-preview")).toHaveLength(
        1,
      );
      expect(preview()).toHaveAttribute("data-drop-target", target);
      expect(store.getState().workspace).toBe(before);
    }
    expect(screen.getByText("Join tabs")).toBeInTheDocument();
    expect(screen.queryByText("Split")).not.toBeInTheDocument();
    const dropEvent = new MouseEvent("drop", {
      clientX: 390,
      clientY: 150,
      bubbles: true,
      cancelable: true,
    });
    Object.defineProperty(dropEvent, "dataTransfer", { value: transfer() });
    fireEvent(pane, dropEvent);
    expect(preview()).not.toBeInTheDocument();
    const root = store.getState().workspace.groups["chat:a"]?.root;
    expect(root?.kind).toBe("split");
    expect(root?.kind === "split" && root.dir).toBe("row");
  });

  it.each(["escape", "dragend", "drop", "blur", "leave"])(
    "clears native preview on %s without a layout change",
    (end) => {
      const { pane, store } = setup();
      const before = store.getState().workspace;
      native(pane, 10, 150);
      expect(preview()).toBeInTheDocument();
      if (end === "escape") fireEvent.keyDown(window, { key: "Escape" });
      else if (end === "leave") fireEvent.dragLeave(pane);
      else fireEvent(window, new Event(end));
      expect(preview()).not.toBeInTheDocument();
      expect(store.getState().workspace).toBe(before);
    },
  );

  it("rejects self, unsupported and untyped text payloads", () => {
    const { pane, store } = setup();
    const before = store.getState().workspace;
    for (const dataTransfer of [
      transfer("chat:a"),
      transfer("dashboard"),
      transfer("chat:b", false),
    ]) {
      native(pane, 200, 150, dataTransfer);
      expect(preview()).not.toBeInTheDocument();
      const event = new MouseEvent("drop", {
        clientX: 200,
        clientY: 150,
        bubbles: true,
      });
      Object.defineProperty(event, "dataTransfer", { value: dataTransfer });
      fireEvent(pane, event);
      expect(store.getState().workspace).toBe(before);
    }
  });

  it("uses protected global source identity and excludes file header actions", () => {
    const { pane } = setup();
    expect(screen.queryByRole("button", { name: "Pane actions" })).toBeNull();
    fireEvent.dragStart(screen.getByText("Drag B"), {
      dataTransfer: transfer(),
    });
    const protectedData = transfer();
    protectedData.getData = () => "";
    native(pane, 390, 150, protectedData);
    expect(preview()).toHaveAttribute("data-drop-target", "right");
    const controls = pane.querySelector<HTMLElement>("[data-pane-controls]");
    if (!controls) throw new Error("Missing floating controls");
    controls.style.opacity = "1";
    controls.getBoundingClientRect = () =>
      ({
        left: 350,
        right: 400,
        top: 0,
        bottom: 30,
        width: 50,
        height: 30,
      }) as DOMRect;
    native(pane, 375, 15, protectedData);
    expect(preview()).not.toBeInTheDocument();
    native(pane, 200, 15, protectedData);
    expect(preview()).toHaveAttribute("data-drop-target", "top");
    expect(preview()?.parentElement?.style.top).toBe("");
    expect(pane.querySelector('[role="tablist"]')).toBeNull();
    controls.style.opacity = "0";
    native(pane, 375, 15, protectedData);
    expect(preview()).toHaveAttribute("data-drop-target", "top");
  });
});

describe("single pointer pane preview", () => {
  it("keeps the pane zone registered through edge-center switches and drops at the final actual point", () => {
    const { store } = setup("jetbrains");
    const before = store.getState().workspace;
    const registration = vi.spyOn(pointerDragController, "registerZone");
    act(() =>
      pointerDragController.startDrag(
        { payload: { type: "chat", id: "b" } },
        { x: 10, y: 150 },
      ),
    );
    for (const [x, y, target] of [
      [10, 150, "left"],
      [200, 150, "center"],
      [400, 0, "right"],
      [200, 10, "top"],
    ] as const) {
      pointer("pointermove", x, y);
      expect(screen.getAllByTestId("workspace-pane-drop-preview")).toHaveLength(
        1,
      );
      expect(preview()).toHaveAttribute("data-drop-target", target);
      expect(store.getState().workspace).toBe(before);
    }
    expect(registration).not.toHaveBeenCalled();
    registration.mockRestore();
    pointer("pointerup", 200, 290);
    expect(preview()).not.toBeInTheDocument();
    const root = store.getState().workspace.groups["chat:a"]?.root;
    expect(root?.kind === "split" && root.dir).toBe("col");
    expect(
      root?.kind === "split" &&
        root.children[1].kind === "leaf" &&
        root.children[1].tabIds,
    ).toContain("chat:b");
    expect(screen.getByLabelText("Workspace pane root")).toBeInTheDocument();
  });

  it.each(["Escape", "pointercancel", "blur", "outside"])(
    "cancels or leaves on %s without mutation",
    (end) => {
      const { store } = setup("jetbrains");
      const before = store.getState().workspace;
      act(() =>
        pointerDragController.startDrag(
          { payload: { type: "chat", id: "b" } },
          { x: 10, y: 150 },
        ),
      );
      expect(preview()).toBeInTheDocument();
      if (end === "Escape") fireEvent.keyDown(window, { key: "Escape" });
      else if (end === "outside") pointer("pointermove", 500, 500);
      else fireEvent(window, new Event(end));
      expect(preview()).not.toBeInTheDocument();
      expect(store.getState().workspace).toBe(before);
    },
  );

  it("does not preview or mutate for self or unsupported pointer payloads", () => {
    const { store } = setup("jetbrains");
    const before = store.getState().workspace;
    for (const payload of [
      { type: "chat", id: "a" },
      { type: "surface", id: "dashboard", surfaceKey: "dashboard" },
    ] as const) {
      act(() =>
        pointerDragController.startDrag({ payload }, { x: 200, y: 150 }),
      );
      expect(preview()).not.toBeInTheDocument();
      pointer("pointerup", 200, 150);
      expect(store.getState().workspace).toBe(before);
    }
  });
});
