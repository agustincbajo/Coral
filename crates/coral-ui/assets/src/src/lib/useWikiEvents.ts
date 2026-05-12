import { useEffect, useRef } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { useToast } from "@/components/ui/toaster";
import { getApiBase } from "@/lib/config";

// NOTE(coral-ui frontend): EventSource is enough here — the events
// endpoint is unauthenticated, GET, and the SPA only consumes side-
// effects (cache invalidation + a small toast). We pair it with an
// exponential backoff loop and a toast throttle so rapid wiki churn
// doesn't spam the user.

const TOAST_THROTTLE_MS = 5_000;
const INITIAL_BACKOFF_MS = 1_000;
const MAX_BACKOFF_MS = 30_000;
const TIMEOUT_RECONNECT_MS = 5_000;

const INVALIDATE_KEYS = [
  ["pages"],
  ["graph"],
  ["stats"],
  ["interfaces"],
  ["contract_status"],
] as const;

/**
 * Opens an `/api/v1/events` stream and invalidates the relevant
 * react-query caches whenever the wiki changes. Mount once at the
 * AppRoot — multiple mounts would each open their own stream.
 */
export function useWikiEvents() {
  const qc = useQueryClient();
  const toast = useToast();
  const { t } = useTranslation();
  // Refs so the inner callbacks observe the latest mutators without
  // forcing an effect re-run (which would close+reopen the stream).
  const toastRef = useRef(toast);
  const tRef = useRef(t);
  toastRef.current = toast;
  tRef.current = t;

  useEffect(() => {
    if (typeof window === "undefined" || typeof EventSource === "undefined") {
      return;
    }

    let es: EventSource | null = null;
    let backoff = INITIAL_BACKOFF_MS;
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let timeoutTimer: ReturnType<typeof setTimeout> | null = null;
    let lastToastAt = 0;
    let cancelled = false;

    const clearTimers = () => {
      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      if (timeoutTimer) {
        clearTimeout(timeoutTimer);
        timeoutTimer = null;
      }
    };

    const scheduleReconnect = (delay: number) => {
      clearTimers();
      if (cancelled) return;
      reconnectTimer = setTimeout(() => {
        connect();
      }, delay);
    };

    const handleWikiChanged = () => {
      // Invalidate the affected queries — TanStack will refetch any
      // that are currently observed (mounted views).
      for (const key of INVALIDATE_KEYS) {
        qc.invalidateQueries({ queryKey: key as unknown as readonly unknown[] });
      }
      const now = Date.now();
      if (now - lastToastAt >= TOAST_THROTTLE_MS) {
        lastToastAt = now;
        toastRef.current({
          title: tRef.current("events.wiki_updated_toast"),
          variant: "default",
        });
      }
    };

    const handleTimeout = () => {
      // Backend says "timed out; please reconnect" — we close and
      // re-open after a small delay. This is distinct from a
      // network-level error.
      es?.close();
      es = null;
      scheduleReconnect(TIMEOUT_RECONNECT_MS);
    };

    const connect = () => {
      if (cancelled) return;
      const base = getApiBase().replace(/\/$/, "");
      const url = `${base}/events`;
      try {
        es = new EventSource(url);
      } catch {
        // Browsers in privacy modes may reject EventSource — give up
        // silently rather than spamming reconnects.
        return;
      }

      es.addEventListener("hello", () => {
        // Successful (re)connection; reset backoff.
        backoff = INITIAL_BACKOFF_MS;
      });
      es.addEventListener("wiki_changed", handleWikiChanged);
      es.addEventListener("timeout", handleTimeout);

      es.onerror = () => {
        // EventSource will auto-reconnect on its own, but its strategy
        // is opaque — we close + reopen with an explicit exponential
        // backoff so the cadence is predictable.
        es?.close();
        es = null;
        const next = backoff;
        backoff = Math.min(MAX_BACKOFF_MS, backoff * 2);
        scheduleReconnect(next);
      };
    };

    connect();

    return () => {
      cancelled = true;
      clearTimers();
      es?.close();
      es = null;
    };
  }, [qc]);
}
