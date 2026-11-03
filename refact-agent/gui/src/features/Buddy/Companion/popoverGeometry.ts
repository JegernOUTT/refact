export type Geometry = { x: number; y: number; width: number; height: number };

export const POPOVER_MARGIN = 8;
export const POPOVER_MIN_WIDTH = 320;
export const POPOVER_MIN_HEIGHT = 360;
export const POPOVER_DEFAULT_WIDTH = 420;
export const POPOVER_DEFAULT_HEIGHT = 600;
export const POPOVER_DEFAULT_HEIGHT_RATIO = 0.75;
export const POPOVER_NARROW_BREAKPOINT = 600;

export type Viewport = { width: number; height: number };

export function currentViewport(): Viewport {
  return { width: window.innerWidth, height: window.innerHeight };
}

export function clampGeometry(value: Geometry, viewport: Viewport): Geometry {
  const maxWidth = Math.max(1, viewport.width - 2 * POPOVER_MARGIN);
  const maxHeight = Math.max(1, viewport.height - 2 * POPOVER_MARGIN);
  const width = Math.min(
    Math.max(Math.min(POPOVER_MIN_WIDTH, maxWidth), value.width),
    maxWidth,
  );
  const height = Math.min(
    Math.max(Math.min(POPOVER_MIN_HEIGHT, maxHeight), value.height),
    maxHeight,
  );
  return {
    x: Math.min(
      Math.max(POPOVER_MARGIN, value.x),
      viewport.width - width - POPOVER_MARGIN,
    ),
    y: Math.min(
      Math.max(POPOVER_MARGIN, value.y),
      viewport.height - height - POPOVER_MARGIN,
    ),
    width,
    height,
  };
}

export function defaultGeometry(
  viewport: Viewport,
  anchor: { right: number; bottom: number } | null,
): Geometry {
  const height = Math.min(
    POPOVER_DEFAULT_HEIGHT,
    viewport.height * POPOVER_DEFAULT_HEIGHT_RATIO,
  );
  const bottom = Math.min(
    anchor?.bottom ?? viewport.height - POPOVER_MARGIN,
    viewport.height - POPOVER_MARGIN,
  );
  const left = (anchor?.right ?? 0) + POPOVER_MARGIN;
  const available = viewport.width - left - POPOVER_MARGIN;
  return clampGeometry(
    {
      x: left,
      y: bottom - height,
      width:
        viewport.width < POPOVER_NARROW_BREAKPOINT
          ? available
          : Math.min(POPOVER_DEFAULT_WIDTH, available),
      height,
    },
    viewport,
  );
}

export function movedGeometry(
  origin: Geometry,
  dx: number,
  dy: number,
  viewport: Viewport,
): Geometry {
  return clampGeometry(
    { ...origin, x: origin.x + dx, y: origin.y + dy },
    viewport,
  );
}

export function resizedGeometry(
  origin: Geometry,
  dx: number,
  dy: number,
  viewport: Viewport,
): Geometry {
  return clampGeometry(
    {
      ...origin,
      width: Math.min(
        origin.width + dx,
        viewport.width - origin.x - POPOVER_MARGIN,
      ),
      height: Math.min(
        origin.height + dy,
        viewport.height - origin.y - POPOVER_MARGIN,
      ),
    },
    viewport,
  );
}

function isGeometry(value: unknown): value is Geometry {
  if (!value || typeof value !== "object") return false;
  const record = value as Record<string, unknown>;
  return ["x", "y", "width", "height"].every(
    (key) => typeof record[key] === "number" && Number.isFinite(record[key]),
  );
}

export function popoverStorageKey(namespace: string) {
  return `refact:buddy:popover:v1:${namespace}`;
}

export function readStoredGeometry(key: string): Geometry | null {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return null;
    const value: unknown = JSON.parse(raw);
    return isGeometry(value) ? value : null;
  } catch {
    return null;
  }
}

export function writeStoredGeometry(key: string, value: Geometry) {
  try {
    localStorage.setItem(key, JSON.stringify(value));
  } catch {
    return;
  }
}
