/**
 * Hook that handles mount/unmount with animations.
 * Returns { shouldRender, isAnimatingOpen } where:
 * - shouldRender: true while content should be in DOM (including during animations)
 * - isAnimatingOpen: true when the open animation should be applied (delayed by 1 frame on mount)
 *
 * @param isOpen - Whether the content should be visible
 * @param delayMs - How long to wait before unmounting (should match animation duration)
 * @param animate - Whether to animate the transition (when false, state changes are instant)
 */
export declare function useDelayedUnmount(isOpen: boolean, delayMs?: number, animate?: boolean): {
    shouldRender: boolean;
    isAnimatingOpen: boolean;
};
