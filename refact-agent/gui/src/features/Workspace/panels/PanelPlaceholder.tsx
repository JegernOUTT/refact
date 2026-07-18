import { Files, GitBranch, type LucideIcon } from "lucide-react";

import { EmptyState } from "../../../components/ui";
import type { PanelKind } from "../surfaceKey";

type PlaceholderPanelKind = Exclude<PanelKind, "terminal">;

type PanelDefinition = {
  icon: LucideIcon;
  title: string;
};

const PANEL_DEFINITIONS: Record<PlaceholderPanelKind, PanelDefinition> = {
  files: { icon: Files, title: "Files" },
  git: { icon: GitBranch, title: "Git" },
};

export function PanelPlaceholder({ kind }: { kind: PlaceholderPanelKind }) {
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
export { FilesPanel };
