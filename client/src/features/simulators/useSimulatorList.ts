import {
  startTransition,
  useCallback,
  useEffect,
  useRef,
  useState,
} from "react";

import { listSimulators } from "../../api/simulators";
import type { SimulatorMetadata } from "../../api/types";

const LOCAL_REFRESH_MS = 30000;
const REMOTE_REFRESH_MS = 60000;
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
      startTransition(() => setSimulators(bootedFirst(nextSimulators)));
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

  const updateSimulator = useCallback((nextSimulator: SimulatorMetadata) => {
    startTransition(() =>
      setSimulators((current) => {
        let replaced = false;
        const nextSimulators = current.map((simulator) => {
          if (simulator.udid !== nextSimulator.udid) {
            return simulator;
          }
          replaced = true;
          return {
            ...simulator,
            ...nextSimulator,
          };
        });
        return bootedFirst(replaced ? nextSimulators : [nextSimulator, ...current]);
      }),
    );
  }, []);

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

    const refreshWhenVisible = () => {
      if (!cancelled && document.visibilityState === "visible") {
        if (timeoutId) {
          window.clearTimeout(timeoutId);
          timeoutId = 0;
        }
        run();
      }
    };

    run();
    document.addEventListener("visibilitychange", refreshWhenVisible);
    window.addEventListener("focus", refreshWhenVisible);

    return () => {
      cancelled = true;
      if (timeoutId) {
        window.clearTimeout(timeoutId);
      }
      document.removeEventListener("visibilitychange", refreshWhenVisible);
      window.removeEventListener("focus", refreshWhenVisible);
    };
  }, [remote]);

  return {
    error,
    isLoading,
    refresh,
    simulators,
    updateSimulator,
  };
}

function bootedFirst(simulators: SimulatorMetadata[]): SimulatorMetadata[] {
  return simulators
    .map((simulator, index) => ({ simulator, index }))
    .sort((left, right) => {
      if (left.simulator.isBooted !== right.simulator.isBooted) {
        return left.simulator.isBooted ? -1 : 1;
      }
      return left.index - right.index;
    })
    .map(({ simulator }) => simulator);
}
