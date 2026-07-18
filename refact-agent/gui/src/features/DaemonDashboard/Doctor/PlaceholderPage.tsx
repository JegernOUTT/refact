import { Stethoscope } from "lucide-react";

import { EmptyState } from "../../../components/ui";

export function DoctorPlaceholderPage() {
  return (
    <EmptyState
      icon={Stethoscope}
      title="Doctor"
      description="System checks and guided fixes will appear here."
    />
  );
}
