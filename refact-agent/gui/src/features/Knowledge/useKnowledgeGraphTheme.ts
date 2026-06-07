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

  const color = (name: string) => tokens[name] || `var()`;

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
