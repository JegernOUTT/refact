import React from "react";
import { Theme as RadixTheme } from "@radix-ui/themes";
import "@radix-ui/themes/styles.css";
import "../../styles/tokens.css";
import "../../styles/motion.css";
import "../../styles/responsive.css";
import "../../styles/scrollbar.css";
import "./theme-config.css";
import "../shared/tokens.css";
import { useAppearance, useConfig } from "../../hooks";

export type ThemeProps = React.ComponentPropsWithoutRef<typeof RadixTheme>;

export const Theme = React.forwardRef<HTMLDivElement, ThemeProps>(
  (props, ref) => {
    const { host, themeProps } = useConfig();
    const { appearance } = useAppearance();

    return (
      <RadixTheme
        {...themeProps}
        {...props}
        ref={ref}
        appearance={appearance}
        data-host={host}
        data-appearance={appearance}
      />
    );
  },
);

Theme.displayName = "Theme";
