import { humanizeIdentifier } from "../../utils/displayNames";

export function headline(title: string): string {
  const trimmed = title.trim();
  return (
    trimmed
      .split(/\r?\n/)
      .map((line) => line.trim())
      .find(Boolean) ?? trimmed
  );
}

export function sameText(a: string, b: string): boolean {
  const normalize = (value: string) =>
    value.trim().replace(/\s+/g, " ").toLowerCase();
  return normalize(a) === normalize(b);
}

export function computeXpFill(xp: number, xpNext: number): number {
  if (!Number.isFinite(xp)) return 0;
  if (!Number.isFinite(xpNext)) return xp > 0 ? 100 : 0;
  if (xpNext <= 0) return xp > 0 ? 100 : 0;
  return Math.min(100, Math.max(0, (xp / xpNext) * 100));
}

export function xpDisplay(
  xp: number,
  xpNext: number | undefined,
  atMaxStage: boolean,
): string {
  if (atMaxStage || xpNext === undefined || xpNext <= 0) {
    return `${xp} XP · MAX`;
  }
  return `${Math.min(xp, xpNext)} / ${xpNext} XP`;
}

export function anxietyFromNeglect(neglectScore: number): number {
  return Math.min(100, Math.max(0, Math.round(neglectScore / 2)));
}

export function humanizeResourceReference(
  resourceKind: string,
  resourceId: string,
): string {
  const opaque =
    /^producer:/i.test(resourceId) ||
    /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i.test(
      resourceId,
    ) ||
    /[0-9a-f]{24,}/i.test(resourceId);
  return humanizeIdentifier(opaque ? resourceKind : resourceId);
}

export function formatCompactNumber(value: number): string {
  if (!Number.isFinite(value)) return "0";
  const abs = Math.abs(value);
  const sign = value < 0 ? "-" : "";
  if (abs < 1000) return Math.trunc(value).toString();
  const units: { threshold: number; suffix: string }[] = [
    { threshold: 1e12, suffix: "T" },
    { threshold: 1e9, suffix: "B" },
    { threshold: 1e6, suffix: "M" },
    { threshold: 1e3, suffix: "k" },
  ];
  for (const u of units) {
    if (abs >= u.threshold) {
      const scaled = abs / u.threshold;
      const formatted =
        scaled >= 100
          ? Math.round(scaled).toString()
          : scaled.toFixed(1).replace(/\.0$/, "");
      return `${sign}${formatted}${u.suffix}`;
    }
  }
  return `${sign}${abs}`;
}
