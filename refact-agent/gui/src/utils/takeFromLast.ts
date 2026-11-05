function lastIndex<A>(arr: A[], predicate: (a: A) => boolean): number {
  for (let index = arr.length - 1; index >= 0; index--) {
    if (predicate(arr[index])) return index;
  }
  return -1;
}

export function takeFromLast<A>(arr: A[], predicate: (a: A) => boolean): A[] {
  const start = lastIndex(arr, predicate);
  if (start === -1) return [];
  return arr.slice(start + 1);
}
