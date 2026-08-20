import { useState, useCallback } from "react";

const STORAGE_KEY = "dashboard:v1:collapse";

type CollapseState = {
  buddy: boolean;
};

const DEFAULTS: CollapseState = {
  buddy: false,
};

function isBool(x: unknown): x is boolean {
  return typeof x === "boolean";
}

// Parsing stays lenient: older builds persisted `chats`/`tasks` keys next to
// `buddy`. Unknown keys are ignored instead of invalidating the whole entry.
function load(): CollapseState {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) {
      const parsed = JSON.parse(raw) as Record<string, unknown>;
      return {
        buddy: isBool(parsed.buddy) ? parsed.buddy : DEFAULTS.buddy,
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
