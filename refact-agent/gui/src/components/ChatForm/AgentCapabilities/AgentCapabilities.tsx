import { Flex } from "@radix-ui/themes";
import { UsageCounter } from "../../UsageCounter";
import { useUsageCounter } from "../../UsageCounter/useUsageCounter";
import { TrajectoryButton } from "../../Trajectory";

export type AgentCapabilitiesProps = {
  trajectoryOpen?: boolean;
  onTrajectoryOpenChange?: (open: boolean) => void;
};

export const AgentCapabilities = ({
  trajectoryOpen,
  onTrajectoryOpenChange,
}: AgentCapabilitiesProps) => {
  const { shouldShow: shouldShowUsage } = useUsageCounter();

  if (!shouldShowUsage) {
    return null;
  }

  return (
    <Flex mb="2" gap="2" align="center" justify="end">
      <Flex align="center" gap="1">
        <UsageCounter />
        <TrajectoryButton
          forceOpen={trajectoryOpen}
          onOpenChange={onTrajectoryOpenChange}
        />
      </Flex>
    </Flex>
  );
};
