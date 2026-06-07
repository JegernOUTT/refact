import { GitBranch } from "lucide-react";
import type { ComponentProps, FC } from "react";

export const BranchIcon: FC<ComponentProps<typeof GitBranch>> = (props) => (
  <GitBranch aria-hidden="true" strokeWidth={1.5} {...props} />
);
