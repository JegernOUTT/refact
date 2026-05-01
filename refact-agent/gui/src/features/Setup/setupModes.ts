export interface SetupMode {
  label: string;
  mode: string;
}

export const SETUP_MODES: SetupMode[] = [
  { label: "Run Setup", mode: "setup" },
  { label: "Create Skills", mode: "setup_skills" },
  { label: "Setup AGENTS.md", mode: "setup_agents_md" },
  { label: "Find MCPs", mode: "setup_mcp" },
  { label: "Create Commands", mode: "setup_commands" },
  { label: "Create Subagents", mode: "setup_subagents" },
];

export const SETUP_MODE_IDS = new Set(SETUP_MODES.map((m) => m.mode));

export function isValidSetupMode(mode: string): boolean {
  return SETUP_MODE_IDS.has(mode);
}
