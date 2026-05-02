import { startTransition, useEffect, useRef, useState } from "react";

import { listSimulators } from "../../api/simulators";
import type { SimulatorMetadata } from "../../api/types";

const LOCAL_REFRESH_MS = 5000;
const REMOTE_REFRESH_MS = 10000;
const REMOTE_ERROR_REFRESH_MS = 15000;
const REMOTE_REQUEST_TIMEOUT_MS = 12000;

interface UseSimulatorListOptions {
  remote?: boolean;
}

export function useSimulatorList({
  remote = false,
}: UseSimulatorListOptions = {}) {
  const [simulators, setSimulators] = useState<SimulatorMetadata[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState("");
  const inFlightRef = useRef(false);
  const lastLoadFailedRef = useRef(false);

  async function loadSimulators(cancelled = false) {
    if (inFlightRef.current) {
      return;
    }
    inFlightRef.current = true;
    const controller =
      remote && typeof AbortController !== "undefined"
        ? new AbortController()
        : null;
    const timeoutId = controller
      ? window.setTimeout(() => controller.abort(), REMOTE_REQUEST_TIMEOUT_MS)
      : 0;
    try {
      const nextSimulators = await listSimulators(
        controller ? { signal: controller.signal } : {},
      );
      if (cancelled) {
        return;
      }
      startTransition(() => setSimulators(nextSimulators));
      setError("");
      lastLoadFailedRef.current = false;
    } catch (loadError) {
      if (!cancelled) {
        setError(
          loadError instanceof DOMException && loadError.name === "AbortError"
            ? "Timed out waiting for provider."
            : loadError instanceof Error
              ? loadError.message
              : "Failed to load simulators.",
        );
        lastLoadFailedRef.current = true;
      }
    } finally {
      if (timeoutId) {
        window.clearTimeout(timeoutId);
      }
      inFlightRef.current = false;
      if (!cancelled) {
        setIsLoading(false);
      }
    }
  }

  async function refresh() {
    await loadSimulators();
  }

  useEffect(() => {
    let cancelled = false;
    let timeoutId = 0;

    const scheduleNext = () => {
      if (cancelled) {
        return;
      }
      const delay = remote
        ? lastLoadFailedRef.current
          ? REMOTE_ERROR_REFRESH_MS
          : REMOTE_REFRESH_MS
        : LOCAL_REFRESH_MS;
      timeoutId = window.setTimeout(run, delay);
    };

    const run = () => {
      void loadSimulators(cancelled).finally(scheduleNext);
    };

    run();

    return () => {
      cancelled = true;
      if (timeoutId) {
        window.clearTimeout(timeoutId);
      }
    };
  }, [remote]);

  return {
    error,
    isLoading,
    refresh,
    simulators,
  };
}
