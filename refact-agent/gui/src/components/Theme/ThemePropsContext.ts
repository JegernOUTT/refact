import { createContext } from "react";
import type { Theme as RadixTheme } from "@radix-ui/themes";
import type React from "react";

export type ResolvedThemeProps = {
  host: string;
  themeProps: Partial<React.ComponentPropsWithoutRef<typeof RadixTheme>>;
  appearance: "light" | "dark" | "inherit" | undefined;
};

export const ThemePropsContext = createContext<ResolvedThemeProps | null>(null);
