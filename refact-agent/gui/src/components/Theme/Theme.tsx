import React, { useMemo } from "react";
import { Theme as RadixTheme } from "@radix-ui/themes";
import "@radix-ui/themes/styles.css";
import "../../styles/tokens.css";
import "../../styles/glass.css";
import "../../styles/motion.css";
import "../../styles/responsive.css";
import "../../styles/scrollbar.css";
import "./theme-config.css";
import "../shared/tokens.css";
import { useAppearance, useConfig } from "../../hooks";
import {
  ThemePropsContext,
  type ResolvedThemeProps,
} from "./ThemePropsContext";

export type ThemeProps = React.ComponentPropsWithoutRef<typeof RadixTheme>;

export const Theme = React.forwardRef<HTMLDivElement, ThemeProps>(
  (props, ref) => {
    const { host, themeProps } = useConfig();
    const { appearance } = useAppearance();

    const resolved = useMemo<ResolvedThemeProps>(
      () => ({ host, themeProps, appearance }),
      [host, themeProps, appearance],
    );

    return (
      <ThemePropsContext.Provider value={resolved}>
        <RadixTheme
          {...themeProps}
          {...props}
          ref={ref}
          appearance={appearance}
          data-host={host}
          data-appearance={appearance}
        />
      </ThemePropsContext.Provider>
    );
  },
);

Theme.displayName = "Theme";
