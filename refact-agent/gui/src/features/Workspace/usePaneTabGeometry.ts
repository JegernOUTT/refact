import { useLayoutEffect, useRef, useState } from "react";

type Bounds = { left: number; right: number; top: number; bottom: number };
export type PaneTabBounds = Bounds & { id: string };
export type PaneTabSection = { id: string; left: number; width: number };

export function projectPaneTabGeometry(
  slot: Bounds,
  panes: PaneTabBounds[],
): PaneTabSection[] | null {
  if (slot.right <= slot.left || panes.length === 0) return null;
  const ordered = [...panes].sort((a, b) => a.left - b.left);
  const first = ordered[0];
  if (
    ordered.some(
      (pane, index) =>
        ![pane.left, pane.right, pane.top, pane.bottom].every(
          Number.isFinite,
        ) ||
        pane.right <= pane.left ||
        pane.bottom <= pane.top ||
        Math.abs(pane.top - first.top) > 1 ||
        Math.abs(pane.bottom - first.bottom) > 1 ||
        (index > 0 && pane.left < ordered[index - 1].right - 1),
    )
  )
    return null;
  const sections = ordered.map((pane, index) => {
    const left = Math.max(slot.left, pane.left);
    const right = Math.min(slot.right, ordered[index + 1]?.left ?? pane.right);
    return { id: pane.id, left: left - slot.left, width: right - left };
  });
  return sections.some((section) => section.width < 60) ? null : sections;
}

export function usePaneTabGeometry(
  groupId: string | null,
  leafSignature: string,
) {
  const slotRef = useRef<HTMLDivElement>(null);
  const [snapshot, setSnapshot] = useState<{
    source: string;
    sections: PaneTabSection[] | null;
  }>();
  const source = JSON.stringify([groupId, leafSignature]);
  useLayoutEffect(() => {
    if (!groupId) return;
    let frame = 0;
    let retry: ReturnType<typeof setTimeout> | undefined;
    let observer: ResizeObserver | undefined;
    let root: HTMLElement | undefined;
    let panes: HTMLElement[] = [];
    const ids = JSON.parse(leafSignature) as string[];
    const bind = () => {
      root = Array.from(
        document.querySelectorAll<HTMLElement>("[data-workspace-group-tab-id]"),
      ).find((element) => element.dataset.workspaceGroupTabId === groupId);
      panes = root
        ? Array.from(
            root.querySelectorAll<HTMLElement>("[data-workspace-leaf-id]"),
          ).filter((pane) => ids.includes(pane.dataset.workspaceLeafId ?? ""))
        : [];
      observer?.disconnect();
      if (slotRef.current) observer?.observe(slotRef.current);
      if (root) observer?.observe(root);
      panes.forEach((pane) => observer?.observe(pane));
    };
    const measure = () => {
      frame = 0;
      if (
        !root?.isConnected ||
        panes.some((pane) => !pane.isConnected) ||
        panes.length !== ids.length
      )
        bind();
      const slot = slotRef.current;
      const sections =
        slot && root?.isConnected && panes.length === ids.length
          ? projectPaneTabGeometry(
              slot.getBoundingClientRect(),
              panes.map((pane) => {
                const { left, right, top, bottom } =
                  pane.getBoundingClientRect();
                return {
                  id: pane.dataset.workspaceLeafId ?? "",
                  left,
                  right,
                  top,
                  bottom,
                };
              }),
            )
          : null;
      setSnapshot((previous) =>
        previous?.source === source &&
        JSON.stringify(previous.sections) === JSON.stringify(sections)
          ? previous
          : { source, sections },
      );
      if (!root?.isConnected || panes.length !== ids.length) {
        retry = setTimeout(() => {
          retry = undefined;
          schedule();
        }, 100);
      }
    };
    const schedule = () => {
      if (retry !== undefined) {
        clearTimeout(retry);
        retry = undefined;
      }
      if (!frame) frame = requestAnimationFrame(measure);
    };
    frame = requestAnimationFrame(() => {
      frame = 0;
      observer = new ResizeObserver(schedule);
      measure();
    });
    window.addEventListener("resize", schedule);
    return () => {
      cancelAnimationFrame(frame);
      if (retry !== undefined) clearTimeout(retry);
      observer?.disconnect();
      window.removeEventListener("resize", schedule);
    };
  }, [groupId, leafSignature, source]);
  return {
    slotRef,
    sections: snapshot?.source === source ? snapshot.sections : null,
  };
}
