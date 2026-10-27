const FORBIDDEN_CHARACTERS = /[\s~^:?*[\\\p{Cc}]/u;

export function branchNameError(
  name: string,
  existing: readonly string[] = [],
): string | null {
  if (name.length === 0) return "Enter a branch name.";
  if (
    name === "@" ||
    FORBIDDEN_CHARACTERS.test(name) ||
    name.includes("..") ||
    name.includes("//") ||
    name.includes("@{") ||
    name.startsWith("-") ||
    name.startsWith("/") ||
    name.endsWith("/") ||
    name.endsWith(".") ||
    name.endsWith(".lock") ||
    name.split("/").some((part) => part.startsWith("."))
  ) {
    return "This is not a valid Git branch name.";
  }
  if (existing.includes(name)) return `Branch '${name}' already exists.`;
  return null;
}

export function remoteOfRef(
  ref: string,
  remotes: readonly string[],
): { remote: string; branch: string } | null {
  const known = [...remotes]
    .sort((left, right) => right.length - left.length)
    .find((remote) => ref.startsWith(`${remote}/`));
  if (known) return { remote: known, branch: ref.slice(known.length + 1) };
  const slash = ref.indexOf("/");
  if (slash <= 0) return null;
  return { remote: ref.slice(0, slash), branch: ref.slice(slash + 1) };
}
