import React, { useContext } from "react";
import { createPortal } from "react-dom";
import { Theme as RadixTheme } from "@radix-ui/themes";
import { ThemePropsContext } from "../Theme/ThemePropsContext";

export type PortalProps = React.ComponentPropsWithoutRef<typeof RadixTheme> & {
  element?: HTMLElement;
};

export const Portal = React.forwardRef<HTMLDivElement, PortalProps>(
  ({ children, element = document.body, ...props }, ref) => {
    const resolved = useContext(ThemePropsContext);

    return createPortal(
      <RadixTheme
        {...resolved?.themeProps}
        {...props}
        ref={ref}
        appearance={resolved?.appearance}
        data-host={resolved?.host}
        data-appearance={resolved?.appearance}
      >
        {children}
      </RadixTheme>,
      element,
    );
  },
);

Portal.displayName = "Portal";
