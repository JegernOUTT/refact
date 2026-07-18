import { Activity } from "lucide-react";

import { EmptyState } from "../../../components/ui";

export function ActivityPlaceholderPage() {
  return (
    <EmptyState
      icon={Activity}
      title="Activity"
      description="Daemon events and logs will appear here."
    />
  );
}
