import type React from "react";

export function chipKeyHandler(
  action: () => void,
): (event: React.KeyboardEvent) => void {
  return (event) => {
    if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      action();
    }
  };
}
