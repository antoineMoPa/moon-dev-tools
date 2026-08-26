import { createContext, useContext } from "react";

/// The review used to scroll the page itself. Now that it is one pane among others, it
/// scrolls inside its own element, and the parts that used to talk to `window` talk to this.
export type ReviewScrollValue = {
  scrollToTop: () => void;
  /// The element the review scrolls in - what `window` used to be for scroll events and
  /// for deciding which hunk is on screen.
  scrollElement: () => HTMLElement | null;
};

const ReviewScrollContext = createContext<ReviewScrollValue | null>(null);

export const ReviewScrollProvider = ReviewScrollContext.Provider;

export function useReviewScroll() {
  const value = useContext(ReviewScrollContext);
  if (!value) {
    throw new Error("useReviewScroll must be used within the review pane");
  }
  return value;
}
