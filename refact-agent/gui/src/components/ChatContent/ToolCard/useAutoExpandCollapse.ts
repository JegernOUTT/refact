import { useState, useEffect, useCallback, useRef } from "react";
import { useCollapsibleStore } from "../useStoredOpen";

export type ToolStatus = "running" | "success" | "error";

interface UseAutoExpandCollapseOptions {
  status: ToolStatus;
  collapseDelayMs?: number;
  storeKey?: string;
}

interface UseAutoExpandCollapseResult {
  isOpen: boolean;
  onToggle: () => void;
  animate: boolean;
}

export function useAutoExpandCollapse({
  status,
  collapseDelayMs = 500,
  storeKey,
}: UseAutoExpandCollapseOptions): UseAutoExpandCollapseResult {
  const store = useCollapsibleStore();
  const initialOpen = storeKey && store ? store.get(storeKey) : undefined;

  const [isOpen, setIsOpen] = useState(initialOpen ?? status === "running");
  const [animate, setAnimate] = useState(false);
  const userToggledRef = useRef(false);
  const prevStatusRef = useRef(status);
  const finalizedRef = useRef(status !== "running");
  const collapseTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    if (storeKey && store) store.set(storeKey, isOpen);
  }, [storeKey, store, isOpen]);

  useEffect(() => {
    if (finalizedRef.current) {
      return;
    }

    if (status === "running" && prevStatusRef.current !== "running") {
      if (!userToggledRef.current) {
        setAnimate(false);
        setIsOpen(true);
      }
    }

    if (status !== "running" && prevStatusRef.current === "running") {
      finalizedRef.current = true;
      if (userToggledRef.current) {
        prevStatusRef.current = status;
        return;
      }
      collapseTimerRef.current = setTimeout(() => {
        collapseTimerRef.current = null;
        setAnimate(false);
        setIsOpen(false);
        userToggledRef.current = false;
      }, collapseDelayMs);
      prevStatusRef.current = status;
      return () => {
        if (collapseTimerRef.current !== null) {
          clearTimeout(collapseTimerRef.current);
          collapseTimerRef.current = null;
        }
      };
    }

    prevStatusRef.current = status;
  }, [status, collapseDelayMs]);

  const onToggle = useCallback(() => {
    userToggledRef.current = true;
    if (collapseTimerRef.current !== null) {
      clearTimeout(collapseTimerRef.current);
      collapseTimerRef.current = null;
    }
    setAnimate(true);
    setIsOpen((prev) => !prev);
  }, []);

  return { isOpen, onToggle, animate };
}
