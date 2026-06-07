import { useTokens } from "../../components/ui";

export function useKnowledgeGraphTheme() {
  const tokens = useTokens([
    "--rf-surface-1",
    "--rf-surface-2",
    "--rf-color-accent",
    "--rf-color-accent-soft",
    "--rf-color-fg",
    "--rf-color-muted",
    "--rf-color-faint",
    "--rf-color-success",
    "--rf-color-warning",
    "--rf-color-info",
    "--rf-color-danger",
    "--rf-border-strong",
  ]);

  const color = (name: string, fallback: string) => tokens[name] || fallback;

  const colors = {
    surface: color("--rf-surface-1", "rgba(255, 255, 255, 0.035)"),
    panel: color("--rf-surface-2", "rgba(255, 255, 255, 0.06)"),
    accent: color("--rf-color-accent", "#6f8bff"),
    accentSoft: color("--rf-color-accent-soft", "rgba(111, 139, 255, 0.16)"),
    foreground: color("--rf-color-fg", "rgba(255, 255, 255, 0.92)"),
    muted: color("--rf-color-muted", "rgba(255, 255, 255, 0.54)"),
    faint: color("--rf-color-faint", "rgba(255, 255, 255, 0.34)"),
    border: color("--rf-border-strong", "rgba(255, 255, 255, 0.14)"),
    kind: {
      code: color("--rf-color-info", "#6f8bff"),
      decision: color("--rf-color-accent", "#6f8bff"),
      preference: color("--rf-color-success", "#5fae8b"),
      pattern: color("--rf-color-warning", "#cda04e"),
      lesson: color("--rf-color-info", "#6f8bff"),
      trajectory: color("--rf-color-faint", "rgba(255, 255, 255, 0.34)"),
      other: color("--rf-color-muted", "rgba(255, 255, 255, 0.54)"),
    },
    status: {
      active: color("--rf-color-accent", "#6f8bff"),
      deprecated: color("--rf-color-danger", "#d8736d"),
      archived: color("--rf-color-faint", "rgba(255, 255, 255, 0.34)"),
    },
  };

  const isDark =
    document.documentElement.getAttribute("data-appearance") === "dark" ||
    document.documentElement.classList.contains("dark");

  return { colors, isDark };
}
