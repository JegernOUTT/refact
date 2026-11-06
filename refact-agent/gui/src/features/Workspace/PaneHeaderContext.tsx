import { createContext, useContext, useState, type ReactNode } from "react";

const HeaderTargetContext = createContext<HTMLElement | null | undefined>(
  undefined,
);
const HeaderSetterContext = createContext<
  ((node: HTMLElement | null) => void) | null
>(null);
const SurfaceActivityContext = createContext(true);
const SurfaceFocusedContext = createContext(true);

export function PaneHeaderProvider({ children }: { children: ReactNode }) {
  const [target, setTarget] = useState<HTMLElement | null>(null);
  return (
    <HeaderSetterContext.Provider value={setTarget}>
      <HeaderTargetContext.Provider value={target}>
        {children}
      </HeaderTargetContext.Provider>
    </HeaderSetterContext.Provider>
  );
}

export function PaneHeaderActions({ className }: { className?: string }) {
  const setTarget = useContext(HeaderSetterContext);
  return <div className={className} ref={setTarget} />;
}

// Context hooks intentionally share this provider module.
// eslint-disable-next-line react-refresh/only-export-components
export function usePaneHeaderTarget(): HTMLElement | null | undefined {
  return useContext(HeaderTargetContext);
}

export function PaneSurfaceActivityProvider({
  active,
  focused = true,
  children,
}: {
  active: boolean;
  focused?: boolean;
  children: ReactNode;
}) {
  return (
    <SurfaceActivityContext.Provider value={active}>
      <SurfaceFocusedContext.Provider value={focused}>
        {children}
      </SurfaceFocusedContext.Provider>
    </SurfaceActivityContext.Provider>
  );
}

// eslint-disable-next-line react-refresh/only-export-components
export function usePaneSurfaceActive(): boolean {
  return useContext(SurfaceActivityContext);
}

// eslint-disable-next-line react-refresh/only-export-components
export function usePaneSurfaceFocused(): boolean {
  return useContext(SurfaceFocusedContext);
}
