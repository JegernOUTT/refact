import { PALETTES } from "../constants";
import type { ColorMap } from "../types";

export function buildColorMap(paletteIndex: number): ColorMap {
  const p = PALETTES[paletteIndex] ?? PALETTES[0];
  return {
    body: p.body,
    light: p.light,
    dark: p.dark,
    belly: p.belly,
    outline: p.outline,
    eyeDark: p.eyeDark,
    black: "#000",
    white: "#FFF",
    rosy: p.rosy,
    accent: p.accent,
    green: "#22C55E",
    gold: "#FFD700",
  };
}
