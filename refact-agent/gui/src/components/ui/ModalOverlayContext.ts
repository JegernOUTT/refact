import React from "react";

const ModalOverlayContext = React.createContext(false);

export const ModalOverlayProvider = ModalOverlayContext.Provider;

export function useIsInsideModalOverlay() {
  return React.useContext(ModalOverlayContext);
}
