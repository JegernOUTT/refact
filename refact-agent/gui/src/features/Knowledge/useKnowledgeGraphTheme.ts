export function useKnowledgeGraphTheme() {
  const isDark =
    document.documentElement.getAttribute("data-appearance") === "dark" ||
    document.documentElement.classList.contains("dark");

  const colors = {
    surface: "var(--color-surface)",
    panel: "var(--color-panel)",
    accent: "var(--accent-9)",
    gray: "var(--gray-9)",

    kind: {
      code: "#3b82f6",
      decision: "#8b5cf6",
      trajectory: "#6b7280",
      preference: "#10b981",
      other: "#6b7280",
    },

    status: {
      active: "var(--accent-9)",
      deprecated: "#ef4444",
      archived: "#9ca3af",
    },
  };

  return { colors, isDark };
}
