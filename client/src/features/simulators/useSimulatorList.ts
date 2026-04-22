import { startTransition, useEffect, useState } from "react";

import { listSimulators } from "../../api/simulators";
import type { SimulatorMetadata } from "../../api/types";

export function useSimulatorList() {
  const [simulators, setSimulators] = useState<SimulatorMetadata[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState("");

  async function refresh() {
    const nextSimulators = await listSimulators();
    startTransition(() => setSimulators(nextSimulators));
    setError("");
  }

  useEffect(() => {
    let cancelled = false;

    async function load() {
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

    void load();
    const intervalId = window.setInterval(() => {
      void load();
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
