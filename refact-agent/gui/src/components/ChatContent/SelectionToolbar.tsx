import React, { useCallback, useEffect, useState } from "react";
import { Tooltip } from "@radix-ui/themes";
import { CopyIcon, ChatBubbleIcon } from "@radix-ui/react-icons";
import { addInputValue } from "../ChatForm/actions";
import styles from "./SelectionToolbar.module.css";

type ToolbarState = {
  text: string;
  top: number;
  left: number;
} | null;

export const SelectionToolbar: React.FC = () => {
  const [state, setState] = useState<ToolbarState>(null);

  const onMouseUp = useCallback((e: MouseEvent) => {
    const target = e.target as HTMLElement;
    if (target.closest("[data-selection-toolbar]")) return;
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || !sel.rangeCount) {
      setState(null);
      return;
    }
    const text = sel.toString().trim();
    if (!text) {
      setState(null);
      return;
    }
    const range = sel.getRangeAt(0);
    const anchor = sel.anchorNode;
    const el = anchor
      ? anchor.nodeType === Node.ELEMENT_NODE
        ? (anchor as Element)
        : anchor.parentElement
      : null;
    if (el?.closest("textarea, input, [contenteditable]")) {
      setState(null);
      return;
    }
    const rect = range.getBoundingClientRect();
    if (rect.width === 0 && rect.height === 0) {
      setState(null);
      return;
    }
    setState({
      text,
      top: Math.max(8, rect.top - 44),
      left: rect.left + rect.width / 2,
    });
  }, []);

  const onMouseDown = useCallback((e: MouseEvent) => {
    const target = e.target as HTMLElement;
    if (target.closest("[data-selection-toolbar]")) return;
    setState(null);
  }, []);

  useEffect(() => {
    document.addEventListener("mouseup", onMouseUp);
    document.addEventListener("mousedown", onMouseDown);
    return () => {
      document.removeEventListener("mouseup", onMouseUp);
      document.removeEventListener("mousedown", onMouseDown);
    };
  }, [onMouseUp, onMouseDown]);

  const onCopy = useCallback(() => {
    if (!state) return;
    void navigator.clipboard.writeText(state.text);
    setState(null);
  }, [state]);

  const onReply = useCallback(() => {
    if (!state) return;
    const quoted = state.text
      .split("\n")
      .map((line) => `> ${line}`)
      .join("\n");
    const textarea = document.querySelector<HTMLTextAreaElement>(
      '[data-testid="chat-form-textarea"]',
    );
    const current = textarea?.value ?? "";
    const prefix = current.length > 0 && !current.endsWith("\n") ? "\n" : "";
    window.postMessage(
      addInputValue({
        value: `${prefix}${quoted}\n\n`,
        send_immediately: false,
      }),
      "*",
    );
    setState(null);
    setTimeout(() => {
      if (textarea) {
        textarea.focus();
        textarea.setSelectionRange(
          textarea.value.length,
          textarea.value.length,
        );
      }
    }, 50);
  }, [state]);

  if (!state) return null;

  return (
    <div
      data-selection-toolbar=""
      className={styles.toolbar}
      style={{ top: state.top, left: state.left }}
    >
      <Tooltip content="Copy">
        <div className={styles.item} onClick={onCopy}>
          <CopyIcon />
        </div>
      </Tooltip>
      <Tooltip content="Reply with quote">
        <div className={styles.item} onClick={onReply}>
          <ChatBubbleIcon />
        </div>
      </Tooltip>
    </div>
  );
};
