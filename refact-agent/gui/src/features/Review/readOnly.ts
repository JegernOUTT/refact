import { createContext, useContext } from "react";

/** True while the canvas shows stored data that could not be refreshed. */
export const ReviewReadOnlyContext = createContext(false);

export function useReviewReadOnly(): boolean {
  return useContext(ReviewReadOnlyContext);
}
