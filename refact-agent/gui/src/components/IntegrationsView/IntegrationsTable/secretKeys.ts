const SECRET_KEY_PATTERN =
  /token|secret|password|passwd|apikey|api_key|credential|auth|bearer|private/i;

export function isSecretKeyName(key: string): boolean {
  const words = key
    .replace(/([a-z0-9])([A-Z])/g, "$1_$2")
    .toLowerCase()
    .split(/[^a-z0-9]+/)
    .filter(Boolean);

  return (
    SECRET_KEY_PATTERN.test(key) ||
    words.includes("key") ||
    words.includes("pat")
  );
}
