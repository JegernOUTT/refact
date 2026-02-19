import React from "react";
import { HoverCard, Text } from "@radix-ui/themes";

export interface CircularProgressProps {
  done: number;
  total: number;
  failed?: number;
  size?: number;
}

export const CircularProgress: React.FC<CircularProgressProps> = ({
  done,
  total,
  failed = 0,
  size = 16,
}) => {
  const hasError = failed > 0;
  const progress = total > 0 ? done / total : 0;
  const strokeWidth = 2;
  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const strokeDashoffset = circumference * (1 - progress);

  const progressColor = hasError ? "var(--red-9)" : "var(--green-9)";
  const trackColor = "var(--gray-6)";

  const tooltip = hasError
    ? `${done}/${total} completed, ${failed} failed`
    : `${done}/${total} completed`;

  return (
    <HoverCard.Root openDelay={200} closeDelay={100}>
      <HoverCard.Trigger>
        <svg
          width={size}
          height={size}
          viewBox={`0 0 ${size} ${size}`}
          style={{ transform: "rotate(-90deg)", flexShrink: 0 }}
          aria-label={tooltip}
        >
          {/* Background track */}
          <circle
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            stroke={trackColor}
            strokeWidth={strokeWidth}
          />
          {/* Progress arc */}
          <circle
            cx={size / 2}
            cy={size / 2}
            r={radius}
            fill="none"
            stroke={progressColor}
            strokeWidth={strokeWidth}
            strokeDasharray={circumference}
            strokeDashoffset={strokeDashoffset}
            strokeLinecap="round"
            style={{ transition: "stroke-dashoffset 0.3s ease" }}
          />
        </svg>
      </HoverCard.Trigger>
      <HoverCard.Content size="1" side="top" align="center">
        <Text as="p" size="1">
          {tooltip}
        </Text>
      </HoverCard.Content>
    </HoverCard.Root>
  );
};
