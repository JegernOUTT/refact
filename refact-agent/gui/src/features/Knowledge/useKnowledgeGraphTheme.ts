import { useTokens } from "../../components/ui";

const FALLBACK_COLORS: Record<string, string> = {
  "--rf-surface-1": "rgba(255, 255, 255, 0.72)",
  "--rf-surface-2": "rgba(255, 255, 255, 0.88)",
  "--rf-color-accent": "#2563eb",
  "--rf-color-accent-soft": "rgba(37, 99, 235, 0.18)",
  "--rf-color-fg": "#111827",
  "--rf-color-muted": "#6b7280",
  "--rf-color-faint": "#9ca3af",
  "--rf-color-success": "#16a34a",
  "--rf-color-warning": "#d97706",
  "--rf-color-info": "#0ea5e9",
  "--rf-color-danger": "#dc2626",
  "--rf-border-strong": "#9ca3af",
};

function isConcreteColor(value: string): boolean {
  const color = value.trim();
  return Boolean(color) && !color.includes("var(");
}

export function useKnowledgeGraphTheme() {
  const tokens = useTokens(Object.keys(FALLBACK_COLORS));

  const color = (name: string) => {
    const token = tokens[name];
    if (token && isConcreteColor(token)) return token;
    return FALLBACK_COLORS[name];
  };

  const colors = {
    surface: color("--rf-surface-1"),
    panel: color("--rf-surface-2"),
    accent: color("--rf-color-accent"),
    accentSoft: color("--rf-color-accent-soft"),
    foreground: color("--rf-color-fg"),
    muted: color("--rf-color-muted"),
    faint: color("--rf-color-faint"),
    border: color("--rf-border-strong"),
    kind: {
      code: color("--rf-color-info"),
      decision: color("--rf-color-accent"),
      preference: color("--rf-color-success"),
      pattern: color("--rf-color-warning"),
      lesson: color("--rf-color-info"),
      trajectory: color("--rf-color-faint"),
      other: color("--rf-color-muted"),
    },
    status: {
      active: color("--rf-color-accent"),
      deprecated: color("--rf-color-danger"),
      archived: color("--rf-color-faint"),
    },
  };

  const isDark =
    document.documentElement.getAttribute("data-appearance") === "dark" ||
    document.documentElement.classList.contains("dark");

  return { colors, isDark };
}
