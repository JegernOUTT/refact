import { useCallback, useEffect, useState } from "react";
import { useAppDispatch } from "./useAppDispatch";
import { useConfig } from "./useConfig";
import { setThemeMode } from "../features/Config/configSlice";
import { useMutationObserver } from "./useMutationObserver";

export type ResolvedAppearance = "light" | "dark";

const BODY_OBSERVER_OPTIONS: MutationObserverInit = {
  attributes: true,
  characterData: false,
  childList: false,
  subtree: false,
};

function detectBodyAppearance(): ResolvedAppearance | null {
  if (typeof document === "undefined") return null;
  const cl = document.body.classList;
  if (cl.contains("vscode-dark") || cl.contains("vscode-high-contrast")) {
    return "dark";
  }
  if (
    cl.contains("vscode-light") ||
    cl.contains("vscode-high-contrast-light")
  ) {
    return "light";
  }
  return null;
}

function detectSystemDark(): boolean {
  if (typeof window === "undefined") return false;
  return window.matchMedia("(prefers-color-scheme: dark)").matches;
}

// Resolves the stored appearance preference ("inherit" included) to a concrete
// value consumers can render with. The stored preference itself is never
// mutated by resolution, so "inherit" survives as the user's setting.
export function resolveConcreteAppearance(
  raw: "inherit" | "light" | "dark" | undefined,
  systemDark: boolean,
): ResolvedAppearance {
  if (raw === "dark" || raw === "light") return raw;
  const fromBody = detectBodyAppearance();
  if (fromBody) return fromBody;
  return systemDark ? "dark" : "light";
}

export const useAppearance = () => {
  const config = useConfig();
  const dispatch = useAppDispatch();

  const rawAppearance = config.themeProps.appearance;
  const [systemDark, setSystemDark] = useState(detectSystemDark);
  const [, setBodyTick] = useState(0);

  useEffect(() => {
    if (typeof window === "undefined") return;
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = (event: MediaQueryListEvent) =>
      setSystemDark(event.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  // Host theme changes (e.g. VSCode flipping its theme updates body classes)
  // only trigger a local re-resolution; they never dispatch a theme mode, so
  // an explicit "inherit" choice is preserved in the store.
  const bumpBodyTick = useCallback(() => setBodyTick((t) => t + 1), []);
  useMutationObserver(document.body, bumpBodyTick, BODY_OBSERVER_OPTIONS);

  const appearance = resolveConcreteAppearance(rawAppearance, systemDark);

  const toggle = useCallback(() => {
    dispatch(setThemeMode(appearance === "dark" ? "light" : "dark"));
  }, [appearance, dispatch]);

  return {
    appearance,
    setAppearance: setThemeMode,
    isDarkMode: appearance === "dark",
    toggle,
  };
};
