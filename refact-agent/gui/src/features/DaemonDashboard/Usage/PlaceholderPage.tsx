import { ChartNoAxesCombined } from "lucide-react";

import { EmptyState } from "../../../components/ui";

export function UsagePlaceholderPage() {
  return (
    <EmptyState
      icon={ChartNoAxesCombined}
      title="Usage"
      description="Model and project usage will appear here."
    />
  );
}
