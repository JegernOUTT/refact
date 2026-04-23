import React from "react";

type Segment = {
  value: number;
  color: string;
  label: string;
};

type MiniDonutProps = {
  segments: Segment[];
  size?: number;
  strokeWidth?: number;
};

export const MiniDonut: React.FC<MiniDonutProps> = ({
  segments,
  size = 36,
  strokeWidth = 5,
}) => {
  const total = segments.reduce((s, seg) => s + seg.value, 0);
  if (total === 0) return null;

  const radius = (size - strokeWidth) / 2;
  const circumference = 2 * Math.PI * radius;
  const cx = size / 2;
  const cy = size / 2;

  let accumulated = 0;

  return (
    <svg width={size} height={size} aria-label="Distribution">
      {segments.map((seg, i) => {
        if (seg.value <= 0) return null;
        const fraction = seg.value / total;
        const offset = accumulated * circumference;
        accumulated += fraction;
        return (
          <circle
            key={`${seg.label}-${i}`}
            cx={cx}
            cy={cy}
            r={radius}
            fill="none"
            stroke={seg.color}
            strokeWidth={strokeWidth}
            strokeDasharray={`${fraction * circumference} ${circumference}`}
            strokeDashoffset={-offset}
            transform={`rotate(-90 ${cx} ${cy})`}
          >
            <title>{`${seg.label}: ${Math.round(fraction * 100)}%`}</title>
          </circle>
        );
      })}
    </svg>
  );
};
