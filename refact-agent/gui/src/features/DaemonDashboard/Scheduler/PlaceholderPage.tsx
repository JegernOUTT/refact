import { CalendarClock } from "lucide-react";

import { EmptyState } from "../../../components/ui";

export function SchedulerPlaceholderPage() {
  return (
    <EmptyState
      icon={CalendarClock}
      title="Scheduler"
      description="Cross-project scheduled work will appear here."
    />
  );
}
