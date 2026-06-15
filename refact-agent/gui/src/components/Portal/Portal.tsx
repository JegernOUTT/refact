import React from "react";
import { createPortal } from "react-dom";
import { useConfig } from "../../hooks";
import { Theme } from "../Theme";

export type PortalProps = React.ComponentPropsWithoutRef<typeof Theme> & {
  element?: HTMLElement;
};

export const Portal = React.forwardRef<HTMLDivElement, PortalProps>(
  ({ children, element = document.body, ...props }, ref) => {
    const config = useConfig();
    return createPortal(
      <Theme {...config.themeProps} {...props} ref={ref}>
        {children}
      </Theme>,
      element,
    );
  },
);

Portal.displayName = "Portal";
