import { Settings } from "lucide-react";

import { EmptyState } from "../../../components/ui";

export function SettingsPlaceholderPage() {
  return (
    <EmptyState
      icon={Settings}
      title="Settings"
      description="Daemon settings and updates will appear here."
    />
  );
}
