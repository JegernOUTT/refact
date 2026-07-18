import { FolderKanban } from "lucide-react";

import { EmptyState } from "../../../components/ui";

export function ProjectsPlaceholderPage() {
  return (
    <EmptyState
      icon={FolderKanban}
      title="Projects"
      description="Project controls and worker health are coming here."
    />
  );
}
