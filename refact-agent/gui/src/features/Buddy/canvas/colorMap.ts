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

export function spriteColorRecord(m: ColorMap): Record<string, string> {
  return {
    B: m.body,
    L: m.light,
    D: m.dark,
    W: m.belly,
    O: m.outline,
    E: m.eyeDark,
    P: m.black,
    H: m.white,
    R: m.rosy,
    A: m.accent,
    G: m.green,
    Y: m.gold,
  };
}
