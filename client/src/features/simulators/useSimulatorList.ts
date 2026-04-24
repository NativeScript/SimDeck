import { startTransition, useEffect, useState } from "react";

import { listSimulators } from "../../api/simulators";
import type { SimulatorMetadata } from "../../api/types";

export function useSimulatorList() {
  const [simulators, setSimulators] = useState<SimulatorMetadata[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState("");

  async function loadSimulators(cancelled = false) {
    try {
      const nextSimulators = await listSimulators();
      if (cancelled) {
        return;
      }
      startTransition(() => setSimulators(nextSimulators));
      setError("");
    } catch (loadError) {
      if (!cancelled) {
        setError(
          loadError instanceof Error
            ? loadError.message
            : "Failed to load simulators.",
        );
      }
    } finally {
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

    void loadSimulators();
    const intervalId = window.setInterval(() => {
      void loadSimulators(cancelled);
    }, 5000);

    return () => {
      cancelled = true;
      clearInterval(intervalId);
    };
  }, []);

  return {
    error,
    isLoading,
    refresh,
    simulators,
  };
}
