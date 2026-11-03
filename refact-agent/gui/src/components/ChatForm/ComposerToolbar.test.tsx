import { useState } from "react";
import { act } from "react-dom/test-utils";
import { afterEach, describe, expect, test, vi } from "vitest";
import { render, screen, waitFor } from "../../utils/test-utils";
import { ComposerToolbar, type ComposerToolbarItem } from "./ComposerToolbar";
import {
  planComposerLayout,
  type ComposerLayoutItem,
  type ComposerRow,
} from "./composerLayout";

const ICON = 28;

function bottomRow(): ComposerLayoutItem[] {
  const icon = (id: string): ComposerLayoutItem => ({
    id,
    row: "bottom",
    pinned: false,
    widths: [ICON],
  });
  return [
    { id: "model", row: "bottom", pinned: false, widths: [210, 160, 120] },
    { id: "mode", row: "bottom", pinned: false, widths: [124, 70, ICON] },
    { id: "worktree", row: "bottom", pinned: false, widths: [65, ICON] },
    { id: "skills", row: "bottom", pinned: false, widths: [70, ICON] },
    icon("browser"),
    icon("enrichment"),
    icon("autoCompact"),
    icon("wand"),
    icon("image"),
    icon("mic"),
    icon("threadInfo"),
    { id: "send", row: "bottom", pinned: true, widths: [ICON] },
  ];
}

function rows(bottom: number, top = 10_000) {
  const gap = 4;
  return {
    top: { capacity: top, gap },
    bottom: { capacity: bottom, gap },
  } satisfies Record<ComposerRow, { capacity: number; gap: number }>;
}

describe("planComposerLayout", () => {
  test("leaves a fitting row untouched", () => {
    const plan = planComposerLayout(bottomRow(), rows(1_000));
    expect(plan.levels.size).toBe(0);
    expect(plan.hidden.size).toBe(0);
    expect(plan.overflowing.size).toBe(0);
  });

  test("shrinks labels in order before hiding anything", () => {
    const plan = planComposerLayout(bottomRow(), rows(600));
    expect(Object.fromEntries(plan.levels)).toEqual({
      skills: 1,
      worktree: 1,
      mode: 1,
      model: 1,
    });
    expect(plan.hidden.size).toBe(0);
  });

  test("hides one control at a time, rarest first, and stops once the row fits", () => {
    const plan = planComposerLayout(bottomRow(), rows(400));
    expect([...plan.hidden]).toEqual([
      "threadInfo",
      "wand",
      "autoCompact",
      "enrichment",
    ]);
    expect(plan.levels.get("model")).toBe(2);
    expect(plan.overflowing.size).toBe(0);
  });

  test("collapses mode to its icon before hiding it", () => {
    const plan = planComposerLayout(bottomRow(), rows(190));
    expect(plan.levels.get("mode")).toBe(2);
    expect(plan.hidden.has("mode")).toBe(false);
    expect(plan.hidden.has("worktree")).toBe(true);
  });

  test("hides mode last and never hides the model or send", () => {
    const plan = planComposerLayout(bottomRow(), rows(100));
    expect(plan.hidden.has("mode")).toBe(true);
    expect(plan.hidden.has("model")).toBe(false);
    expect(plan.hidden.has("send")).toBe(false);
    expect(plan.levels.get("model")).toBe(2);
    expect(plan.overflowing.has("bottom")).toBe(true);
  });

  test("skips pinned controls", () => {
    const top: ComposerLayoutItem[] = [
      { id: "projectInfo", row: "top", pinned: false, widths: [ICON] },
      { id: "editApproval", row: "top", pinned: false, widths: [ICON] },
      { id: "dangerousApproval", row: "top", pinned: true, widths: [ICON] },
      { id: "quota", row: "top", pinned: false, widths: [102, 16] },
      { id: "context", row: "top", pinned: true, widths: [37] },
      { id: "trajectory", row: "top", pinned: false, widths: [ICON] },
    ];
    const plan = planComposerLayout(top, rows(10_000, 80));
    expect(plan.hidden.has("dangerousApproval")).toBe(false);
    expect(plan.hidden.has("context")).toBe(false);
    expect([...plan.hidden]).toEqual([
      "projectInfo",
      "trajectory",
      "quota",
      "editApproval",
    ]);
  });

  test("pinned controls may shrink but are never hidden", () => {
    const items: ComposerLayoutItem[] = [
      { id: "model", row: "bottom", pinned: false, widths: [100] },
      { id: "skills", row: "bottom", pinned: true, widths: [90, ICON] },
      { id: "send", row: "bottom", pinned: true, widths: [ICON] },
    ];
    const plan = planComposerLayout(items, rows(170));
    expect(plan.levels.get("skills")).toBe(1);
    expect(plan.hidden.has("skills")).toBe(false);
    expect(plan.overflowing.size).toBe(0);
  });

  test("only degrades rows that overflow", () => {
    const items: ComposerLayoutItem[] = [
      { id: "quota", row: "top", pinned: false, widths: [102, 16] },
      { id: "trajectory", row: "top", pinned: false, widths: [ICON] },
      ...bottomRow(),
    ];
    const plan = planComposerLayout(items, rows(400));
    expect(plan.levels.has("quota")).toBe(false);
    expect(plan.hidden.has("trajectory")).toBe(false);
    expect(plan.hidden.has("threadInfo")).toBe(true);
  });

  test("zero-width controls take neither space nor a gap", () => {
    const items: ComposerLayoutItem[] = [
      { id: "model", row: "bottom", pinned: false, widths: [100] },
      { id: "streaming", row: "bottom", pinned: false, widths: [0] },
      { id: "send", row: "bottom", pinned: true, widths: [ICON] },
    ];
    const plan = planComposerLayout(items, rows(132));
    expect(plan.overflowing.size).toBe(0);
    expect(plan.levels.size).toBe(0);
  });
});

function DraftControl() {
  const [draft, setDraft] = useState("");
  return (
    <input
      aria-label="Scope draft"
      value={draft}
      onChange={(event) => setDraft(event.target.value)}
    />
  );
}

type ControlWidths = number[] | ((host: HTMLElement) => number[]);

function mockGeometry(
  widths: Record<string, ControlWidths>,
  rowWidth: () => number,
) {
  const observer: { notify: () => void } = { notify: () => undefined };
  const widthOf = (element: HTMLElement) => {
    const id = element.dataset.composerControl;
    const spec = id ? widths[id] : undefined;
    const levels = typeof spec === "function" ? spec(element) : spec;
    const level = Number(element.dataset.level ?? "0");
    return levels ? levels[Math.min(level, levels.length - 1)] ?? 0 : 0;
  };
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(
    function (this: HTMLElement) {
      return this.dataset.row ? rowWidth() : 0;
    },
  );
  const rect = vi
    .spyOn(HTMLElement.prototype, "getBoundingClientRect")
    .mockImplementation(function (this: HTMLElement) {
      const width = widthOf(this);
      return {
        width,
        height: ICON,
        x: 0,
        y: 0,
        left: 0,
        right: width,
        top: 0,
        bottom: ICON,
        toJSON: () => ({}),
      };
    });
  vi.stubGlobal(
    "ResizeObserver",
    class {
      private targets: HTMLElement[] = [];
      constructor(callback: ResizeObserverCallback) {
        observer.notify = () => {
          const entries = this.targets.map((target) => ({
            target,
            contentRect: {
              width: target.dataset.composerControl
                ? widthOf(target)
                : rowWidth(),
            },
          }));
          callback(
            entries as unknown as ResizeObserverEntry[],
            this as unknown as ResizeObserver,
          );
        };
      }
      observe(target: HTMLElement) {
        this.targets.push(target);
      }
      disconnect() {
        this.targets = [];
      }
    },
  );
  return { notify: () => observer.notify(), rect };
}

async function nextFrame() {
  await act(async () => {
    await new Promise<void>((resolve) => {
      requestAnimationFrame(() => {
        resolve();
      });
    });
  });
}

const host = (id: string) =>
  document.querySelector<HTMLElement>(`[data-composer-control="${id}"]`);

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("ComposerToolbar", () => {
  test("degrades by the sequence, restores in reverse, and keeps control state", async () => {
    let width = 1_000;
    const observer = mockGeometry(
      {
        model: [200, 150, 100],
        mode: [120, 70, ICON],
        worktree: [60, ICON],
        wand: [ICON],
        threadInfo: [ICON],
        send: [ICON],
      },
      () => width,
    );
    const items: ComposerToolbarItem[] = [
      {
        id: "model",
        row: "bottom",
        squeezable: true,
        content: <DraftControl />,
      },
      { id: "mode", row: "bottom", content: <button>Mode</button> },
      { id: "worktree", row: "bottom", content: <button>Worktree</button> },
      {
        id: "wand",
        row: "bottom",
        side: "right",
        content: <button>Wand</button>,
      },
      {
        id: "threadInfo",
        row: "bottom",
        side: "right",
        content: <button>Info</button>,
      },
      {
        id: "send",
        row: "bottom",
        side: "right",
        pinned: true,
        content: <button>Send</button>,
      },
    ];
    const { user } = render(
      <ComposerToolbar
        items={items}
        editor={<textarea aria-label="Editor" />}
        menuCount={0}
        expanded={false}
      />,
    );
    await user.type(screen.getByLabelText("Scope draft"), "kept");
    expect(host("threadInfo")?.dataset.hidden).toBe("false");
    expect(host("wand")?.dataset.firstRight).toBe("true");

    width = 270;
    act(() => observer.notify());
    await waitFor(() =>
      expect(host("threadInfo")?.dataset.hidden).toBe("true"),
    );
    expect(host("wand")?.dataset.hidden).toBe("false");
    expect(host("model")?.dataset.level).toBe("2");
    expect(host("mode")?.dataset.level).toBe("1");
    expect(host("worktree")?.dataset.level).toBe("1");
    expect(screen.getByLabelText("Scope draft")).toHaveValue("kept");

    width = 1_000;
    act(() => observer.notify());
    await waitFor(() =>
      expect(host("threadInfo")?.dataset.hidden).toBe("false"),
    );
    expect(host("model")?.dataset.level).toBe("0");
    expect(screen.getByLabelText("Scope draft")).toHaveValue("kept");
  });

  test("places controls by row and side and reveals the top row only when expanded", () => {
    mockGeometry(
      { projectInfo: [ICON], context: [37], model: [120], send: [ICON] },
      () => 1_000,
    );
    const items: ComposerToolbarItem[] = [
      { id: "projectInfo", row: "top", content: <button>Project</button> },
      {
        id: "context",
        row: "top",
        side: "right",
        content: <button>Ctx</button>,
      },
      { id: "model", row: "bottom", content: <button>Model</button> },
      {
        id: "send",
        row: "bottom",
        side: "right",
        pinned: true,
        content: <button>Send</button>,
      },
    ];
    const { rerender } = render(
      <ComposerToolbar
        items={items}
        editor={<textarea aria-label="Editor" />}
        menuCount={0}
        expanded={false}
      />,
    );
    const top = screen.getByTestId("composer-top-toolbar");
    const bottom = screen.getByTestId("composer-toolbar");
    expect(top.contains(host("projectInfo"))).toBe(true);
    expect(bottom.contains(host("model"))).toBe(true);
    expect(host("context")?.dataset.firstRight).toBe("true");
    expect(top.closest("[data-open]")?.getAttribute("data-open")).toBe("false");

    rerender(
      <ComposerToolbar
        items={items}
        editor={<textarea aria-label="Editor" />}
        menuCount={0}
        expanded
      />,
    );
    expect(top.closest("[data-open]")?.getAttribute("data-open")).toBe("true");
  });

  test("re-measures when the label of a shrunk control changes", async () => {
    mockGeometry(
      {
        model: [100],
        skills: (element) => [
          40 + (element.textContent?.length ?? 0) * 5,
          ICON,
        ],
        send: [ICON],
      },
      () => 240,
    );
    const toolbar = (label: string) => (
      <ComposerToolbar
        items={[
          { id: "model", row: "bottom", content: <button>Model</button> },
          {
            id: "skills",
            row: "bottom",
            side: "right",
            content: <button>{label}</button>,
          },
          {
            id: "send",
            row: "bottom",
            side: "right",
            pinned: true,
            content: <button>Send</button>,
          },
        ]}
        editor={<textarea aria-label="Editor" />}
        menuCount={0}
        expanded={false}
      />
    );
    const { rerender } = render(toolbar("a-very-long-skill-name"));
    expect(host("skills")?.dataset.level).toBe("1");

    rerender(toolbar("short"));
    await waitFor(() => expect(host("skills")?.dataset.level).toBe("0"));
  });

  test("notifications without a width change do not re-run the layout", async () => {
    const geometry = mockGeometry({ model: [100], send: [ICON] }, () => 1_000);
    render(
      <ComposerToolbar
        items={[
          { id: "model", row: "bottom", content: <button>Model</button> },
          {
            id: "send",
            row: "bottom",
            side: "right",
            pinned: true,
            content: <button>Send</button>,
          },
        ]}
        editor={<textarea aria-label="Editor" />}
        menuCount={0}
        expanded={false}
      />,
    );
    act(() => geometry.notify());
    await nextFrame();
    const reads = geometry.rect.mock.calls.length;

    act(() => geometry.notify());
    await nextFrame();
    expect(geometry.rect.mock.calls.length).toBe(reads);
  });

  test("an open menu keeps its trigger at its current level", async () => {
    let width = 300;
    const geometry = mockGeometry(
      { model: [200, 150, 100], mode: [120, 70, ICON], send: [ICON] },
      () => width,
    );
    const toolbar = (open: boolean) => (
      <ComposerToolbar
        items={[
          { id: "model", row: "bottom", content: <button>Model</button> },
          {
            id: "mode",
            row: "bottom",
            content: (
              <button aria-haspopup="menu" aria-expanded={open}>
                Mode
              </button>
            ),
          },
          {
            id: "send",
            row: "bottom",
            side: "right",
            pinned: true,
            content: <button>Send</button>,
          },
        ]}
        editor={<textarea aria-label="Editor" />}
        menuCount={open ? 1 : 0}
        expanded
      />
    );
    const { rerender } = render(toolbar(false));
    expect(host("mode")?.dataset.level).toBe("1");
    expect(host("model")?.dataset.level).toBe("0");

    rerender(toolbar(true));
    expect(host("mode")?.dataset.menuOpen).toBe("true");
    expect(host("mode")?.dataset.level).toBe("1");
    expect(host("model")?.dataset.level).toBe("0");

    width = 1_000;
    act(() => geometry.notify());
    await nextFrame();
    expect(host("mode")?.dataset.level).toBe("1");

    rerender(toolbar(false));
    expect(host("mode")?.dataset.level).toBe("0");
  });

  test("an open menu pins its trigger even when its row overflows", () => {
    mockGeometry({ model: [100], threadInfo: [ICON], send: [ICON] }, () => 120);
    const items: ComposerToolbarItem[] = [
      { id: "model", row: "bottom", content: <button>Model</button> },
      {
        id: "threadInfo",
        row: "bottom",
        side: "right",
        content: (
          <button aria-haspopup="dialog" aria-expanded="true">
            Info
          </button>
        ),
      },
      {
        id: "send",
        row: "bottom",
        side: "right",
        pinned: true,
        content: <button>Send</button>,
      },
    ];
    render(
      <ComposerToolbar
        items={items}
        editor={<textarea aria-label="Editor" />}
        menuCount={1}
        expanded
      />,
    );
    expect(host("threadInfo")?.dataset.hidden).toBe("false");
    expect(host("threadInfo")?.dataset.menuOpen).toBe("true");
  });
});
