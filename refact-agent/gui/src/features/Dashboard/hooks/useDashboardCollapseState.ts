import { useState, useCallback } from "react";

const STORAGE_KEY = "dashboard:v1:collapse";

type CollapseState = {
  buddy: boolean;
  chats: boolean;
  tasks: boolean;
};

const DEFAULTS: CollapseState = {
  buddy: false,
  chats: false,
  tasks: false,
};

function isBool(x: unknown): x is boolean {
  return typeof x === "boolean";
}

function load(): CollapseState {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Partial<
        Record<keyof CollapseState, unknown>
      >;
      return {
        buddy: isBool(parsed.buddy) ? parsed.buddy : DEFAULTS.buddy,
        chats: isBool(parsed.chats) ? parsed.chats : DEFAULTS.chats,
        tasks: isBool(parsed.tasks) ? parsed.tasks : DEFAULTS.tasks,
      };
    }
  } catch {
    /* ignore */
  }
  return { ...DEFAULTS };
}

function save(state: CollapseState): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {
    /* ignore */
  }
}

export function useDashboardCollapseState() {
  const [state, setState] = useState<CollapseState>(load);

  const toggle = useCallback((key: keyof CollapseState) => {
    setState((prev) => {
      const next = { ...prev, [key]: !prev[key] };
      save(next);
      return next;
    });
  }, []);

  return { collapsed: state, toggle };
}
