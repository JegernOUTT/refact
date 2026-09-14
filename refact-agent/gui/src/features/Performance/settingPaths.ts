import type {
  TrajectorySettingsConfig,
  TrajectorySettingValue,
} from "../../services/refact/performance";

export function settingValue(
  config: TrajectorySettingsConfig | undefined,
  path: string,
): TrajectorySettingValue | undefined {
  let current: TrajectorySettingValue | undefined = config;
  for (const key of path.split(".")) {
    if (current === null || typeof current !== "object") return undefined;
    if (!Object.prototype.hasOwnProperty.call(current, key)) return undefined;
    current = current[key];
  }
  return current;
}

export function setSettingValue(
  config: TrajectorySettingsConfig,
  path: string,
  value: TrajectorySettingValue,
): TrajectorySettingsConfig {
  const keys = path.split(".");
  if (
    keys.some(
      (key) => !key || ["__proto__", "constructor", "prototype"].includes(key),
    )
  ) {
    return config;
  }
  const replace = (
    current: TrajectorySettingsConfig,
    index: number,
  ): TrajectorySettingsConfig => {
    const key = keys[index];
    if (index === keys.length - 1) return { ...current, [key]: value };
    const nested = current[key];
    const child = nested !== null && typeof nested === "object" ? nested : {};
    return { ...current, [key]: replace(child, index + 1) };
  };
  return replace(config, 0);
}

export function settingsEqual(
  left: TrajectorySettingValue,
  right: TrajectorySettingValue,
): boolean {
  if (left === right) return true;
  if (
    left === null ||
    right === null ||
    typeof left !== "object" ||
    typeof right !== "object"
  ) {
    return false;
  }
  const keys = Object.keys(left);
  return (
    keys.length === Object.keys(right).length &&
    keys.every(
      (key) =>
        Object.prototype.hasOwnProperty.call(right, key) &&
        settingsEqual(left[key], right[key]),
    )
  );
}
