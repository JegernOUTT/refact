import type { BrowserContextOversizeInfo } from "./browserSlice";

export function formatKB(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  return `${Math.round(bytes / 1024)} KB`;
}

export function estimateSize(
  info: BrowserContextOversizeInfo,
  opts: {
    includeActions: boolean;
    includeConsole: boolean;
    includeNetwork: boolean;
    includeMutations: boolean;
    includeScreenshot: boolean;
    lastNActions: number;
    lastNConsole: number;
    lastNNetwork: number;
  },
): number {
  let total = 0;
  if (opts.includeActions && info.action_count > 0) {
    const ratio = Math.min(opts.lastNActions, info.action_count) / info.action_count;
    total += Math.round(info.action_bytes * ratio);
  }
  if (opts.includeConsole && info.console_count > 0) {
    const ratio = Math.min(opts.lastNConsole, info.console_count) / info.console_count;
    total += Math.round(info.console_bytes * ratio);
  }
  if (opts.includeNetwork && info.network_count > 0) {
    const ratio = Math.min(opts.lastNNetwork, info.network_count) / info.network_count;
    total += Math.round(info.network_bytes * ratio);
  }
  if (opts.includeMutations) {
    total += info.mutation_bytes;
  }
  return total;
}
