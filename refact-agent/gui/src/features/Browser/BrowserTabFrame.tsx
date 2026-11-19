import { useEffect, useRef } from "react";
import type { ClipboardEvent, KeyboardEvent, MouseEvent } from "react";
import type {
  ProjectBrowserFrame,
  ProjectBrowserInputFence,
  ProjectBrowserInputRequest,
  ProjectBrowserModifier,
} from "../../services/refact/browser";
import { framePosition } from "./projectBrowserFrame";
import styles from "./ProjectBrowserTabSurface.module.css";

const IGNORED_KEYS = [
  "Control",
  "Meta",
  "Shift",
  "Alt",
  "Unidentified",
  "Dead",
];

export function BrowserTabFrame({
  frame,
  disabled,
  onInput,
}: {
  frame: ProjectBrowserFrame;
  disabled: boolean;
  onInput: (request: ProjectBrowserInputRequest) => void;
}) {
  const image = useRef<HTMLImageElement>(null);

  useEffect(() => {
    const element = image.current;
    const wheel = (event: WheelEvent) => {
      if (!element || disabled || event.ctrlKey || event.metaKey) return;
      const bounds = element.getBoundingClientRect();
      const point = framePosition(frame, bounds, event.clientX, event.clientY);
      if (!point) return;
      event.preventDefault();
      const scale =
        event.deltaMode === 1
          ? 16
          : event.deltaMode === 2
            ? frame.viewport.height
            : 1;
      const delta = (value: number) =>
        Math.min(10000, Math.max(-10000, value * scale));
      onInput({
        runtime_id: frame.runtime_id,
        tab_id: frame.tab_id,
        kind: "wheel",
        ...point,
        dx: delta(event.deltaX),
        dy: delta(event.deltaY),
      });
    };
    element?.addEventListener("wheel", wheel, { passive: false });
    return () => element?.removeEventListener("wheel", wheel);
  }, [disabled, frame, onInput]);

  const fence: ProjectBrowserInputFence = {
    runtime_id: frame.runtime_id,
    tab_id: frame.tab_id,
  };
  const click = (event: MouseEvent<HTMLImageElement>) => {
    if (disabled) return;
    const point = framePosition(
      frame,
      event.currentTarget.getBoundingClientRect(),
      event.clientX,
      event.clientY,
    );
    if (!point || event.button > 2) return;
    event.preventDefault();
    event.currentTarget.focus();
    const button =
      event.button === 0 ? "left" : event.button === 1 ? "middle" : "right";
    onInput({ ...fence, kind: "click", button, ...point });
  };

  const keyDown = (event: KeyboardEvent<HTMLImageElement>) => {
    if (
      disabled ||
      event.nativeEvent.isComposing ||
      event.key === "Tab" ||
      event.key === "Escape" ||
      IGNORED_KEYS.includes(event.key)
    )
      return;
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "v")
      return;
    event.preventDefault();
    event.stopPropagation();
    const modifiers: ProjectBrowserModifier[] = [];
    if (event.altKey) modifiers.push("Alt");
    if (event.ctrlKey) modifiers.push("Ctrl");
    if (event.metaKey) modifiers.push("Meta");
    if (event.shiftKey) modifiers.push("Shift");
    if (
      Array.from(event.key).length === 1 &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey
    ) {
      onInput({ ...fence, kind: "text", text: event.key });
    } else {
      onInput({ ...fence, kind: "key", key: event.key, modifiers });
    }
  };

  const paste = (event: ClipboardEvent<HTMLImageElement>) => {
    if (disabled) return;
    event.preventDefault();
    event.stopPropagation();
    const text = Array.from(event.clipboardData.getData("text/plain"))
      .slice(0, 8192)
      .join("");
    if (text) onInput({ ...fence, kind: "text", text });
  };

  return (
    <img
      ref={image}
      className={styles.frame}
      src={`data:${frame.mime};base64,${frame.data}`}
      alt="Browser page"
      tabIndex={disabled ? -1 : 0}
      draggable={false}
      onClick={click}
      onAuxClick={click}
      onContextMenu={(event) => event.preventDefault()}
      onKeyDown={keyDown}
      onPaste={paste}
    />
  );
}
