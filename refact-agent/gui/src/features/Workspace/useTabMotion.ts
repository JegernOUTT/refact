import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useReducedMotion } from "../../hooks";

type ExitSnapshot<T> = {
  key: string;
  content: T;
  left: number;
  top: number;
  width: number;
  height: number;
};

export function useTabMotion<T = never>(keys: string[], scope = "") {
  const osReduced = useReducedMotion();
  const [htmlReduced, setHtmlReduced] = useState(() =>
    ["on", "true"].includes(
      document.documentElement.dataset.reducedMotion ?? "",
    ),
  );
  const reduced = osReduced || htmlReduced;
  const list = useRef<HTMLDivElement>(null);
  const positions = useRef(new Map<string, number>());
  const pending = useRef(new Map<string, number>());
  const closed = useRef(new Set<string>());
  const [ghosts, setGhosts] = useState<ExitSnapshot<T>[]>([]);
  const signature = JSON.stringify(keys);
  const previousScope = useRef(scope);
  const finish = useCallback((key: string) => {
    const timer = pending.current.get(key);
    if (timer === undefined) return;
    pending.current.delete(key);
    window.clearTimeout(timer);
    setGhosts((current) => current.filter((ghost) => ghost.key !== key));
  }, []);
  const close = (key: string, action: () => void, content?: T) => {
    if (closed.current.has(key)) return;
    closed.current.add(key);
    const instant =
      reduced ||
      ["on", "true"].includes(
        document.documentElement.dataset.reducedMotion ?? "",
      );
    const node = Array.from(list.current?.children ?? []).find(
      (child): child is HTMLElement =>
        child instanceof HTMLElement && child.dataset.paneTabKey === key,
    );
    finish(key);
    if (!instant && node && content !== undefined) {
      const snapshot = {
        key,
        content,
        left: node.offsetLeft,
        top: node.offsetTop,
        width: node.offsetWidth,
        height: node.offsetHeight,
      };
      pending.current.set(
        key,
        window.setTimeout(() => finish(key), 200),
      );
      setGhosts((current) => [...current, snapshot]);
    }
    action();
    queueMicrotask(() => closed.current.delete(key));
  };
  useEffect(() => {
    const observer = new MutationObserver(() => {
      setHtmlReduced(
        ["on", "true"].includes(
          document.documentElement.dataset.reducedMotion ?? "",
        ),
      );
    });
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-reduced-motion"],
    });
    return () => observer.disconnect();
  }, []);
  useEffect(() => {
    if (reduced) for (const key of pending.current.keys()) finish(key);
  }, [finish, reduced]);
  useEffect(() => {
    const node = list.current;
    if (!node) return;
    const complete = (event: Event) => {
      const target = event.target;
      if (target instanceof HTMLElement && target.dataset.paneTabGhost)
        finish(target.dataset.paneTabGhost);
    };
    node.addEventListener("animationend", complete);
    node.addEventListener("animationcancel", complete);
    return () => {
      node.removeEventListener("animationend", complete);
      node.removeEventListener("animationcancel", complete);
    };
  }, [finish]);
  useLayoutEffect(() => {
    if (previousScope.current !== scope) {
      positions.current.clear();
      closed.current.clear();
      previousScope.current = scope;
    }
    const currentKeys = JSON.parse(signature) as string[];
    for (const key of closed.current)
      if (!currentKeys.includes(key)) closed.current.delete(key);
    const elements = Array.from(
      list.current?.querySelectorAll<HTMLElement>("[data-pane-tab-key]") ?? [],
    );
    const next = new Map<string, number>();
    const moving: { element: HTMLElement; delta: number }[] = [];
    for (const element of elements) {
      const key = element.dataset.paneTabKey;
      if (!key) continue;
      const left = element.offsetLeft;
      next.set(key, left);
      const previous = positions.current.get(key);
      if (!reduced && previous !== undefined && previous !== left)
        moving.push({ element, delta: previous - left });
    }
    positions.current = next;
    for (const { element, delta } of moving) {
      element.style.transition = "none";
      element.style.transform = `translateX(${delta}px)`;
    }
    let frame: number | undefined;
    if (moving.length) {
      void list.current?.offsetWidth;
      frame = requestAnimationFrame(() => {
        for (const { element } of moving) {
          element.style.removeProperty("transition");
          element.style.removeProperty("transform");
        }
      });
    }
    return () => {
      if (frame !== undefined) cancelAnimationFrame(frame);
      for (const { element } of moving) {
        element.style.removeProperty("transition");
        element.style.removeProperty("transform");
      }
    };
  }, [signature, reduced, scope]);
  useEffect(() => {
    for (const key of pending.current.keys()) finish(key);
  }, [scope, finish]);
  useEffect(
    () => () => {
      for (const timer of pending.current.values()) window.clearTimeout(timer);
      pending.current.clear();
    },
    [],
  );
  return { list, ghosts, close, reduced };
}
