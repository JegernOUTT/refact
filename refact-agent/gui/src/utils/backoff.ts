export type BackoffOptions = {
  baseDelay?: number;
  maxDelay?: number;
  multiplier?: number;
  jitter?: number;
};

export function calculateBackoff(
  retryCount: number,
  options: BackoffOptions = {},
): number {
  const {
    baseDelay = 1000,
    maxDelay = 30000,
    multiplier = 2,
    jitter = 0.1,
  } = options;

  const delay = Math.min(
    baseDelay * Math.pow(multiplier, retryCount),
    maxDelay,
  );
  const jitterAmount = delay * jitter * (Math.random() * 2 - 1);
  return Math.max(0, Math.round(delay + jitterAmount));
}
