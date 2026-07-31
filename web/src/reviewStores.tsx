import { createContext, useCallback, useContext, useMemo, useState } from "react";
import type { ReactNode } from "react";
import type { ReviewStoreValue } from "./reviewStore";

/// Every review store that is currently mounted, by session. A review store lives inside
/// its own review pane, but the header and the agent monitor sit outside every pane and
/// still need to read the review the user is working in, so each store publishes itself here.
type ReviewStoresValue = {
  stores: Record<string, ReviewStoreValue>;
  publishStore: (sessionId: string, store: ReviewStoreValue | null) => void;
};

const ReviewStoresContext = createContext<ReviewStoresValue | null>(null);

export function ReviewStoresProvider({ children }: { children: ReactNode }) {
  const [stores, setStores] = useState<Record<string, ReviewStoreValue>>({});

  const publishStore = useCallback((sessionId: string, store: ReviewStoreValue | null) => {
    setStores((current) => {
      if (!store) {
        if (!(sessionId in current)) {
          return current;
        }
        const next = { ...current };
        delete next[sessionId];
        return next;
      }
      return { ...current, [sessionId]: store };
    });
  }, []);

  const value = useMemo<ReviewStoresValue>(() => ({ stores, publishStore }), [stores, publishStore]);

  return <ReviewStoresContext.Provider value={value}>{children}</ReviewStoresContext.Provider>;
}

function useReviewStores() {
  const value = useContext(ReviewStoresContext);
  if (!value) {
    throw new Error("useReviewStores must be used within ReviewStoresProvider");
  }
  return value;
}

export function usePublishReviewStore() {
  return useReviewStores().publishStore;
}

/// The store for one session, or null while no review pane is showing it.
export function useReviewStoreFor(sessionId: string | null): ReviewStoreValue | null {
  const { stores } = useReviewStores();
  return sessionId ? (stores[sessionId] ?? null) : null;
}
