import { useMemo } from "react";
import type { BuddyStatusTone } from "../ui";
import type { BuddyNeeds, BuddyPetState } from "../types";

export interface NeedRow {
  key: keyof BuddyNeeds;
  label: string;
  value: number;
  fill: number;
  tone: BuddyStatusTone;
}

interface NeedSpec {
  key: keyof BuddyNeeds;
  label: string;
  highIsBad?: boolean;
  danger: number;
  warning: number;
}

const NEED_SPECS: NeedSpec[] = [
  { key: "hunger", label: "Fullness", danger: 20, warning: 35 },
  { key: "energy", label: "Energy", danger: 20, warning: 35 },
  { key: "hygiene", label: "Hygiene", danger: 25, warning: 40 },
  {
    key: "boredom",
    label: "Boredom",
    highIsBad: true,
    danger: 80,
    warning: 60,
  },
  { key: "affection", label: "Affection", danger: 25, warning: 40 },
];

function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.max(0, Math.min(100, Math.round(value)));
}

function buddyNeedTone(spec: NeedSpec, value: number): BuddyStatusTone {
  if (spec.highIsBad) {
    if (value > spec.danger) return "danger";
    if (value > spec.warning) return "warning";
    return "success";
  }
  if (value < spec.danger) return "danger";
  if (value < spec.warning) return "warning";
  return "success";
}

export function buildBuddyNeedRows(pet: BuddyPetState | undefined): NeedRow[] {
  return NEED_SPECS.map((spec) => {
    const value = clampPercent(pet?.needs[spec.key] ?? 0);
    return {
      key: spec.key,
      label: spec.label,
      value,
      fill: value,
      tone: buddyNeedTone(spec, value),
    };
  });
}

export function useBuddyNeedRows(pet: BuddyPetState | undefined): NeedRow[] {
  return useMemo(() => buildBuddyNeedRows(pet), [pet]);
}
