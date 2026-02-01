import { useState, useEffect, useCallback, useRef } from "react";

export type ToolStatus = "running" | "success" | "error";

interface UseAutoExpandCollapseOptions {
  status: ToolStatus;
  collapseDelayMs?: number;
}

interface UseAutoExpandCollapseResult {
  isOpen: boolean;
  onToggle: () => void;
}

export function useAutoExpandCollapse({
  status,
  collapseDelayMs = 500,
}: UseAutoExpandCollapseOptions): UseAutoExpandCollapseResult {
  const [isOpen, setIsOpen] = useState(status === "running");
  const userToggledRef = useRef(false);
  const prevStatusRef = useRef(status);

  useEffect(() => {
    if (status === "running" && prevStatusRef.current !== "running") {
      if (!userToggledRef.current) {
        setIsOpen(true);
      }
    }

    if (status !== "running" && prevStatusRef.current === "running") {
      const timer = setTimeout(() => {
        setIsOpen(false);
        userToggledRef.current = false;
      }, collapseDelayMs);
      return () => clearTimeout(timer);
    }

    prevStatusRef.current = status;
  }, [status, collapseDelayMs]);

  const onToggle = useCallback(() => {
    userToggledRef.current = true;
    setIsOpen((prev) => !prev);
  }, []);

  return { isOpen, onToggle };
}
