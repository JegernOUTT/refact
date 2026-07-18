import { House } from "lucide-react";

import { EmptyState } from "../../../components/ui";

export function HomePlaceholderPage() {
  return (
    <EmptyState
      icon={House}
      title="Home"
      description="Your daemon overview will appear here."
    />
  );
}
