import type { WebStorage } from "redux-persist";

function removeOldEntry(key: string) {
  try {
    if (
      typeof localStorage !== "undefined" &&
      typeof localStorage.getItem === "function" &&
      typeof localStorage.removeItem === "function" &&
      localStorage.getItem(key)
    ) {
      localStorage.removeItem(key);
    }
  } catch {
    // Storage access can throw in restricted/private contexts; ignore cleanup.
  }
}

function cleanOldEntries() {
  try {
    if (typeof localStorage === "undefined") return;
    removeOldEntry("tipOfTheDay");
    removeOldEntry("chatHistory");
  } catch {
    // Cleanup must never block redux-persist startup.
  }
}

export function storage(): WebStorage {
  cleanOldEntries();
  return {
    getItem(key: string): Promise<string | null> {
      return new Promise((resolve) => {
        try {
          if (
            typeof localStorage === "undefined" ||
            typeof localStorage.getItem !== "function"
          ) {
            resolve(null);
            return;
          }
          resolve(localStorage.getItem(key));
        } catch {
          resolve(null);
        }
      });
    },
    setItem(key: string, item: string): Promise<void> {
      return new Promise((resolve) => {
        try {
          if (
            typeof localStorage !== "undefined" &&
            typeof localStorage.setItem === "function"
          ) {
            localStorage.setItem(key, item);
          }
        } catch {
          // Storage quota exceeded, ignore
        }
        resolve();
      });
    },
    removeItem(key: string): Promise<void> {
      return new Promise((resolve) => {
        try {
          if (
            typeof localStorage !== "undefined" &&
            typeof localStorage.removeItem === "function"
          ) {
            localStorage.removeItem(key);
          }
        } catch {
          // Ignore storage failures
        }
        resolve();
      });
    },
  };
}
