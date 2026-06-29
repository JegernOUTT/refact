import { useEffect, useState } from "react";

type Appearance = "dark" | "light";

export interface KnowledgeGraphColors {
  surface: string;
  panel: string;
  accent: string;
  accentSoft: string;
  foreground: string;
  muted: string;
  faint: string;
  border: string;
  /** Per-memory-kind node colors keyed by the resolved kind string. */
  kind: Record<string, string>;
  /** Color for doc nodes whose kind has no dedicated entry. */
  kindDefault: string;
}

type TokenName =
  | "--rf-surface-1"
  | "--rf-surface-2"
  | "--rf-color-accent"
  | "--rf-color-accent-soft"
  | "--rf-color-fg"
  | "--rf-color-muted"
  | "--rf-color-faint"
  | "--rf-color-success"
  | "--rf-color-warning"
  | "--rf-color-info"
  | "--rf-color-danger"
  | "--rf-border-strong";

// Maps the memory kinds that actually exist in the knowledge graph
// (frontmatter `kind` / backend `node_type`) to semantic design tokens.
// "memory" dominates the graph, so it gets the primary accent; the rarer
// semantic kinds use distinct hues so they stay legible against that mass.
const KIND_TOKENS: Record<string, TokenName> = {
  memory: "--rf-color-accent",
  insight: "--rf-color-info",
  code: "--rf-color-info",
  convention: "--rf-color-success",
  process: "--rf-color-success",
  preference: "--rf-color-success",
  lesson: "--rf-color-success",
  domain: "--rf-color-warning",
  decision: "--rf-color-warning",
  pattern: "--rf-color-warning",
  "task-report": "--rf-color-danger",
  trajectory: "--rf-color-faint",
};

// Concrete fallbacks used for SSR / test environments and whenever a token
// cannot be resolved to a paintable color. They mirror tokens.css so the graph
// stays theme-correct even without a live computed style.
const FALLBACKS: Record<Appearance, Record<TokenName, string>> = {
  dark: {
    "--rf-surface-1": "rgba(255, 255, 255, 0.035)",
    "--rf-surface-2": "rgba(255, 255, 255, 0.045)",
    "--rf-color-accent": "#7f93d8",
    "--rf-color-accent-soft": "rgba(127, 147, 216, 0.4)",
    "--rf-color-fg": "rgba(255, 255, 255, 0.92)",
    "--rf-color-muted": "rgba(255, 255, 255, 0.55)",
    "--rf-color-faint": "rgba(255, 255, 255, 0.32)",
    "--rf-color-success": "#5fae8b",
    "--rf-color-warning": "#cda04e",
    "--rf-color-info": "#7f93d8",
    "--rf-color-danger": "#d8736d",
    "--rf-border-strong": "rgba(255, 255, 255, 0.45)",
  },
  light: {
    "--rf-surface-1": "rgba(0, 0, 0, 0.022)",
    "--rf-surface-2": "rgba(0, 0, 0, 0.05)",
    "--rf-color-accent": "#006adc",
    "--rf-color-accent-soft": "rgba(0, 106, 220, 0.4)",
    "--rf-color-fg": "rgba(0, 0, 0, 0.88)",
    "--rf-color-muted": "rgba(0, 0, 0, 0.55)",
    "--rf-color-faint": "rgba(0, 0, 0, 0.4)",
    "--rf-color-success": "#4f9c79",
    "--rf-color-warning": "#b8862f",
    "--rf-color-info": "#006adc",
    "--rf-color-danger": "#c75b54",
    "--rf-border-strong": "rgba(0, 0, 0, 0.45)",
  },
};

function canUseDOM(): boolean {
  return typeof window !== "undefined" && typeof document !== "undefined";
}

// The active theme lives on the Radix `.radix-themes` root (it carries
// `data-appearance`), not on `document.documentElement`. Reading tokens from
// `<html>` returns the hard-wired dark palette, which is why the graph was
// invisible on the light theme.
function getThemeRoot(): HTMLElement | null {
  if (!canUseDOM()) return null;
  const found = document.querySelector("[data-radix-themes], .radix-themes");
  if (found instanceof HTMLElement) return found;
  return document.documentElement;
}

function isResolvedColor(value: string): boolean {
  const normalized = value.trim().toLowerCase();
  return (
    normalized !== "" &&
    !normalized.includes("var(") &&
    !normalized.includes("color-mix(")
  );
}

// Resolves a CSS custom property to a concrete color that cytoscape's canvas
// can paint. A hidden probe lets the browser fully resolve `var()` and
// `color-mix()` chains; falling back to a direct read and then a literal.
function resolveColor(
  root: HTMLElement | null,
  token: string,
  fallback: string,
): string {
  if (!canUseDOM() || !root) return fallback;

  const probe = document.createElement("span");
  probe.style.color = `var(${token}, ${fallback})`;
  probe.style.display = "none";
  root.appendChild(probe);
  const resolved = window.getComputedStyle(probe).color.trim();
  probe.remove();
  if (isResolvedColor(resolved)) return resolved;

  const direct = window.getComputedStyle(root).getPropertyValue(token).trim();
  if (isResolvedColor(direct)) return direct;

  return fallback;
}

function detectAppearance(root: HTMLElement | null): Appearance {
  if (!canUseDOM()) return "dark";
  const scope = root ?? document.documentElement;
  const themed = scope.closest("[data-appearance]");
  const attr = themed?.getAttribute("data-appearance");
  if (attr === "light" || attr === "dark") return attr;
  if (scope.classList.contains("light")) return "light";
  if (scope.classList.contains("dark")) return "dark";

  const body = document.body.classList;
  if (
    body.contains("vscode-light") ||
    body.contains("vscode-high-contrast-light")
  ) {
    return "light";
  }
  if (body.contains("vscode-dark") || body.contains("vscode-high-contrast")) {
    return "dark";
  }
  if (
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-color-scheme: light)").matches
  ) {
    return "light";
  }
  return "dark";
}

interface ThemeState {
  colors: KnowledgeGraphColors;
  isDark: boolean;
}

function buildColors(useProbe: boolean): ThemeState {
  const root = getThemeRoot();
  const appearance = detectAppearance(root);
  const fallbacks = FALLBACKS[appearance];

  const color = (token: TokenName): string => {
    const fallback = fallbacks[token];
    return useProbe ? resolveColor(root, token, fallback) : fallback;
  };

  const kind: Record<string, string> = {};
  for (const [name, token] of Object.entries(KIND_TOKENS)) {
    kind[name] = color(token);
  }

  const muted = color("--rf-color-muted");
  const colors: KnowledgeGraphColors = {
    surface: color("--rf-surface-1"),
    panel: color("--rf-surface-2"),
    accent: color("--rf-color-accent"),
    accentSoft: color("--rf-color-accent-soft"),
    foreground: color("--rf-color-fg"),
    muted,
    faint: color("--rf-color-faint"),
    border: color("--rf-border-strong"),
    kind,
    kindDefault: muted,
  };

  return { colors, isDark: appearance === "dark" };
}

function collectThemeTargets(): HTMLElement[] {
  if (!canUseDOM()) return [];
  const targets = new Set<HTMLElement>();
  const root = getThemeRoot();
  if (root) targets.add(root);
  targets.add(document.documentElement);
  targets.add(document.body);
  return [...targets];
}

export function useKnowledgeGraphTheme(): ThemeState {
  // Start from concrete fallbacks (never raw `var()`), then resolve against the
  // live themed root once mounted so cytoscape always gets paintable colors.
  const [state, setState] = useState<ThemeState>(() => buildColors(false));

  useEffect(() => {
    if (!canUseDOM()) return;

    const apply = () => setState(buildColors(true));
    apply();

    const observers: MutationObserver[] = [];
    if (typeof MutationObserver !== "undefined") {
      for (const target of collectThemeTargets()) {
        const observer = new MutationObserver(apply);
        observer.observe(target, {
          attributes: true,
          attributeFilter: ["data-appearance", "class"],
        });
        observers.push(observer);
      }
    }

    let mediaQuery: MediaQueryList | null = null;
    if (typeof window.matchMedia === "function") {
      mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
      mediaQuery.addEventListener("change", apply);
    }

    return () => {
      observers.forEach((observer) => observer.disconnect());
      mediaQuery?.removeEventListener("change", apply);
    };
  }, []);

  return state;
}
