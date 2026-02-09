import { useState, useEffect, useCallback, useRef } from "react";

export type ToolStatus = "running" | "success" | "error";

interface UseAutoExpandCollapseOptions {
  status: ToolStatus;
  collapseDelayMs?: number;
}

interface UseAutoExpandCollapseResult {
  isOpen: boolean;
  onToggle: () => void;
  animate: boolean;
}

export function useAutoExpandCollapse({
  status,
  collapseDelayMs = 500,
}: UseAutoExpandCollapseOptions): UseAutoExpandCollapseResult {
  const [isOpen, setIsOpen] = useState(status === "running");
  const [animate, setAnimate] = useState(false);
  const userToggledRef = useRef(false);
  const prevStatusRef = useRef(status);
  const finalizedRef = useRef(status !== "running");

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
      const timer = setTimeout(() => {
        setAnimate(false);
        setIsOpen(false);
        userToggledRef.current = false;
      }, collapseDelayMs);
      prevStatusRef.current = status;
      return () => clearTimeout(timer);
    }

    prevStatusRef.current = status;
  }, [status, collapseDelayMs]);

  const onToggle = useCallback(() => {
    userToggledRef.current = true;
    setAnimate(true);
    setIsOpen((prev) => !prev);
  }, []);

  return { isOpen, onToggle, animate };
}
