import {
  Files,
  GitBranch,
  SquareTerminal,
  type LucideIcon,
} from "lucide-react";

import { EmptyState } from "../../../components/ui";
import type { PanelKind } from "../surfaceKey";

type PanelDefinition = {
  icon: LucideIcon;
  title: string;
};

const PANEL_DEFINITIONS: Record<PanelKind, PanelDefinition> = {
  files: { icon: Files, title: "Files" },
  git: { icon: GitBranch, title: "Git" },
  terminal: { icon: SquareTerminal, title: "Terminal" },
};

export function PanelPlaceholder({ kind }: { kind: PanelKind }) {
  const panel = PANEL_DEFINITIONS[kind];

  return (
    <EmptyState
      icon={panel.icon}
      title={`${panel.title} panel`}
      description="This workspace panel is coming soon."
      variant="full"
    />
  );
}

const FilesPanel = () => <PanelPlaceholder kind="files" />;
const GitPanel = () => <PanelPlaceholder kind="git" />;
const TerminalPanel = () => <PanelPlaceholder kind="terminal" />;

export { FilesPanel, GitPanel, TerminalPanel };
