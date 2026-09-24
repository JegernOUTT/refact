import type { Page } from "@playwright/test";

export type BadgeContrastMeasurement = {
  label: string;
  tone: string | null;
  variant: string | null;
  size: string | null;
  ratio: number;
  required: number;
};

export function measureBadgeContrast(page: Page, selector: string) {
  return page.evaluate((badgeSelector) => {
    type Rgba = { red: number; green: number; blue: number; alpha: number };
    const canvas = document.createElement("canvas");
    canvas.width = 1;
    canvas.height = 1;
    const context = canvas.getContext("2d", { willReadFrequently: true });
    const parseColor = (value: string): Rgba | null => {
      if (!context) return null;
      context.clearRect(0, 0, 1, 1);
      context.fillStyle = value;
      context.fillRect(0, 0, 1, 1);
      const [red, green, blue, alpha] = context.getImageData(0, 0, 1, 1).data;
      return { red, green, blue, alpha: alpha / 255 };
    };
    const blend = (foreground: Rgba, background: Rgba): Rgba => ({
      red:
        foreground.red * foreground.alpha +
        background.red * (1 - foreground.alpha),
      green:
        foreground.green * foreground.alpha +
        background.green * (1 - foreground.alpha),
      blue:
        foreground.blue * foreground.alpha +
        background.blue * (1 - foreground.alpha),
      alpha: 1,
    });
    const luminance = (color: Rgba) => {
      const linear = (channel: number) => {
        const normalized = channel / 255;
        return normalized <= 0.04045
          ? normalized / 12.92
          : ((normalized + 0.055) / 1.055) ** 2.4;
      };
      return (
        0.2126 * linear(color.red) +
        0.7152 * linear(color.green) +
        0.0722 * linear(color.blue)
      );
    };
    const fallback = { red: 255, green: 255, blue: 255, alpha: 1 };

    return [...document.querySelectorAll<HTMLElement>(badgeSelector)].map(
      (element) => {
        const style = getComputedStyle(element);
        const layers: Rgba[] = [];
        let current: HTMLElement | null = element;
        while (current) {
          const background = parseColor(
            getComputedStyle(current).backgroundColor,
          );
          if (background && background.alpha > 0) layers.push(background);
          if (background && background.alpha >= 1) break;
          current = current.parentElement;
        }
        const background = layers
          .reverse()
          .reduce((under, layer) => blend(layer, under), fallback);
        const foreground = parseColor(style.color);
        const compositedForeground = foreground
          ? blend(foreground, background)
          : background;
        const foregroundLuminance = luminance(compositedForeground);
        const backgroundLuminance = luminance(background);
        const ratio =
          (Math.max(foregroundLuminance, backgroundLuminance) + 0.05) /
          (Math.min(foregroundLuminance, backgroundLuminance) + 0.05);
        const fontSize = Number.parseFloat(style.fontSize);
        const fontWeight = Number.parseInt(style.fontWeight, 10);
        const required =
          fontSize >= 24 || (fontSize >= 18.66 && fontWeight >= 700) ? 3 : 4.5;
        return {
          label: (element.textContent ?? "").trim(),
          tone: element.dataset.tone ?? null,
          variant: element.dataset.variant ?? null,
          size: element.dataset.size ?? null,
          ratio: Number(ratio.toFixed(2)),
          required,
        };
      },
    );
  }, selector);
}
