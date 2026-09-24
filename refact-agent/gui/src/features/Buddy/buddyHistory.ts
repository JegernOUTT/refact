export const HISTORY_PAGE_SIZE = 20;

export function countLabel(
  count: number,
  singular: string,
  plural: string,
): string {
  return `${count} ${count === 1 ? singular : plural}`;
}

export function collapseConsecutive<T>(
  entries: T[],
  identity: (entry: T) => string,
): { entry: T; count: number }[] {
  const collapsed: { entry: T; count: number }[] = [];
  for (const entry of entries) {
    const key = identity(entry);
    const previous = collapsed.at(-1);
    if (previous && identity(previous.entry) === key) {
      previous.count += 1;
    } else {
      collapsed.push({ entry, count: 1 });
    }
  }
  return collapsed;
}
