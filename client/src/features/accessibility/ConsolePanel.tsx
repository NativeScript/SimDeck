import type { CSSProperties } from "react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";

import { fetchSimulatorLogs } from "../../api/simulators";
import type {
  AccessibilityNode,
  SimulatorLogEntry,
  SimulatorMetadata,
} from "../../api/types";

interface ConsolePanelProps {
  accessibilityRoots: AccessibilityNode[];
  selectedSimulator: SimulatorMetadata | null;
  visible: boolean;
}

export function ConsolePanel({
  accessibilityRoots,
  selectedSimulator,
  visible,
}: ConsolePanelProps) {
  const [entries, setEntries] = useState<SimulatorLogEntry[]>([]);
  const [error, setError] = useState("");
  const [enabledLevels, setEnabledLevels] = useState<Set<LogLevelGroup>>(
    () => new Set(["debug", "default", "error", "info"]),
  );
  const [selectedProcesses, setSelectedProcesses] = useState<Set<string>>(
    new Set(),
  );
  const [pinnedProcesses, setPinnedProcesses] = useState<Set<string>>(
    new Set(),
  );
  const [serviceMenuOpen, setServiceMenuOpen] = useState(false);
  const [filterText, setFilterText] = useState("");
  const selectedUDID = selectedSimulator?.udid ?? "";
  const selectedIsBooted = Boolean(selectedSimulator?.isBooted);
  const liveLoadingRef = useRef(false);
  const backfillLoadingRef = useRef(false);
  const backfillSignatureRef = useRef("");
  const lastSimulatorUDIDRef = useRef("");
  const preservedScrollTopRef = useRef<number | null>(null);
  const scrollerRef = useRef<HTMLDivElement | null>(null);
  const serviceMenuRef = useRef<HTMLDivElement | null>(null);
  const shouldFollowRef = useRef(true);
  const activePids = collectAccessibilityPids(accessibilityRoots);
  const activeProcesses = collectActiveProcesses(entries, activePids);
  const activeProcessKey = [...activeProcesses].sort().join("\u0000");
  const shownProcesses = new Set([...pinnedProcesses, ...selectedProcesses]);
  const shownProcessList = [...shownProcesses].sort();
  const shownProcessKey = shownProcessList.join("\u0000");
  const enabledLevelList = [...enabledLevels].sort();
  const enabledLevelKey = enabledLevelList.join("\u0000");
  const filterSignature = [
    selectedUDID,
    enabledLevelKey,
    shownProcessKey,
    filterText.trim(),
  ].join("\u0000");
  const availableProcesses = [
    ...new Set(entries.map((entry) => entry.process).filter(Boolean)),
  ].sort((left, right) => left.localeCompare(right));
  const filterNeedle = filterText.trim().toLowerCase();
  const visibleEntries =
    shownProcesses.size > 0
      ? entries.filter((entry) => {
          if (!enabledLevels.has(logLevelGroup(entry.level))) {
            return false;
          }
          if (!shownProcesses.has(entry.process)) {
            return false;
          }
          return (
            !filterNeedle || logEntrySearchText(entry).includes(filterNeedle)
          );
        })
      : [];

  useEffect(() => {
    const udid = selectedUDID;
    if (!udid || lastSimulatorUDIDRef.current === udid) {
      return;
    }

    lastSimulatorUDIDRef.current = udid;
    setEntries(readStoredEntries(udid));
    setError("");
    setSelectedProcesses(new Set());
    setPinnedProcesses(new Set());
    setServiceMenuOpen(false);
    shouldFollowRef.current = true;
    liveLoadingRef.current = false;
    backfillLoadingRef.current = false;
    backfillSignatureRef.current = "";
  }, [selectedUDID]);

  useEffect(() => {
    const udid = selectedUDID;
    if (!udid || lastSimulatorUDIDRef.current !== udid) {
      return;
    }
    writeStoredEntries(udid, entries);
  }, [entries, selectedUDID]);

  useEffect(() => {
    if (activeProcesses.size === 0) {
      return;
    }

    setPinnedProcesses((current) => {
      const next = new Set(current);
      for (const process of activeProcesses) {
        next.add(process);
      }
      return next.size === current.size ? current : next;
    });
  }, [activeProcessKey]);

  useEffect(() => {
    if (!serviceMenuOpen) {
      return;
    }

    function handlePointerDown(event: PointerEvent) {
      if (serviceMenuRef.current?.contains(event.target as Node)) {
        return;
      }
      setServiceMenuOpen(false);
    }

    window.addEventListener("pointerdown", handlePointerDown);
    return () => window.removeEventListener("pointerdown", handlePointerDown);
  }, [serviceMenuOpen]);

  useEffect(() => {
    if (!visible || !selectedIsBooted || !selectedUDID) {
      return;
    }

    async function loadLogs(mode: "backfill" | "live") {
      if (!selectedUDID) {
        return;
      }

      const scroller = scrollerRef.current;
      if (mode === "live" && scroller && !isNearBottom(scroller)) {
        return;
      }

      if (mode === "backfill") {
        if (
          backfillLoadingRef.current ||
          backfillSignatureRef.current === filterSignature
        ) {
          return;
        }
        backfillLoadingRef.current = true;
      } else if (liveLoadingRef.current) {
        return;
      }

      if (mode === "live") {
        liveLoadingRef.current = true;
      }
      try {
        const payload = await fetchSimulatorLogs(selectedUDID, {
          backfill: mode === "backfill",
          levels: enabledLevelList,
          limit: mode === "backfill" ? 1000 : 300,
          processes: shownProcessList,
          query: filterText,
          seconds: mode === "backfill" ? 1800 : 8,
        });
        shouldFollowRef.current = isNearBottom(scroller);
        preservedScrollTopRef.current = scroller?.scrollTop ?? null;
        setEntries((current) =>
          mergeLogEntries(current, payload.entries ?? []),
        );
        if (mode === "backfill") {
          backfillSignatureRef.current = filterSignature;
        }
        setError("");
      } catch (logsError) {
        setError(
          logsError instanceof Error
            ? logsError.message
            : "Failed to read simulator logs.",
        );
      } finally {
        if (mode === "live") {
          liveLoadingRef.current = false;
        }
        if (mode === "backfill") {
          backfillLoadingRef.current = false;
        }
      }
    }

    void (async () => {
      await loadLogs("live");
      void loadLogs("backfill");
    })();
    const interval = window.setInterval(() => {
      void loadLogs("live");
    }, 500);
    return () => window.clearInterval(interval);
  }, [
    enabledLevelKey,
    filterSignature,
    filterText,
    selectedIsBooted,
    selectedUDID,
    shownProcessKey,
    visible,
  ]);

  useLayoutEffect(() => {
    const scroller = scrollerRef.current;
    if (!scroller) {
      return;
    }
    if (shouldFollowRef.current) {
      scroller.scrollTop = scroller.scrollHeight;
      return;
    }
    if (preservedScrollTopRef.current != null) {
      scroller.scrollTop = preservedScrollTopRef.current;
    }
  }, [visibleEntries.length]);

  if (!selectedSimulator) {
    return <div className="hierarchy-empty">Select a simulator.</div>;
  }

  if (!selectedSimulator.isBooted) {
    return (
      <div className="hierarchy-empty">Boot the simulator to view logs.</div>
    );
  }

  if (error) {
    return <div className="hierarchy-empty error">{error}</div>;
  }

  return (
    <>
      <div className="console-filterbar">
        <div className="console-filter-group">
          {LOG_LEVELS.map((level) => (
            <button
              className={`console-filter-chip level-${level} ${enabledLevels.has(level) ? "active" : ""}`}
              key={level}
              onClick={() =>
                setEnabledLevels((current) => {
                  const next = new Set(current);
                  if (next.has(level)) {
                    next.delete(level);
                  } else {
                    next.add(level);
                  }
                  return next;
                })
              }
              type="button"
            >
              {level}
            </button>
          ))}
        </div>
        <div className="console-service-menu-wrap" ref={serviceMenuRef}>
          <button
            className={`console-filter-chip ${serviceMenuOpen ? "active" : ""}`}
            onClick={() => setServiceMenuOpen((current) => !current)}
            type="button"
          >
            Services {shownProcesses.size}/{availableProcesses.length}
          </button>
          {serviceMenuOpen ? (
            <div className="console-service-menu">
              {availableProcesses.length === 0 ? (
                <div className="console-service-empty">No services yet</div>
              ) : (
                availableProcesses.map((process) => (
                  <button
                    className="console-service-option"
                    disabled={pinnedProcesses.has(process)}
                    key={process}
                    onClick={() => {
                      if (pinnedProcesses.has(process)) {
                        return;
                      }
                      setSelectedProcesses((current) => {
                        const next = new Set(current);
                        if (next.has(process)) {
                          next.delete(process);
                        } else {
                          next.add(process);
                        }
                        return next;
                      });
                    }}
                    type="button"
                  >
                    <span
                      className={`console-service-check ${shownProcesses.has(process) ? "checked" : ""}`}
                    />
                    <span className="console-service-name">{process}</span>
                    {pinnedProcesses.has(process) ? (
                      <span className="console-service-pin">active</span>
                    ) : null}
                  </button>
                ))
              )}
            </div>
          ) : null}
        </div>
      </div>
      <div className="console-text-filterbar">
        <input
          className="console-text-filter"
          onChange={(event) => setFilterText(event.target.value)}
          placeholder="Filter logs containing text..."
          type="search"
          value={filterText}
        />
      </div>
      <div
        className="console-panel"
        onScroll={(event) => {
          shouldFollowRef.current = isNearBottom(event.currentTarget);
        }}
        ref={scrollerRef}
      >
        {shownProcesses.size === 0 ? (
          <div className="hierarchy-empty">No active app process detected.</div>
        ) : visibleEntries.length === 0 ? (
          <div className="hierarchy-empty">No matching logs.</div>
        ) : (
          visibleEntries.map((entry) => (
            <div
              className="console-row"
              key={logEntryKey(entry)}
              title={`${formatTimestamp(entry.timestamp)} ${entry.level} ${entry.subsystem}${entry.category ? `:${entry.category}` : ""}`}
            >
              <span
                className="console-process"
                style={
                  {
                    "--console-service-color": serviceColor(entry),
                  } as CSSProperties
                }
              >
                {entry.process}
              </span>
              <span className="console-message">{entry.message}</span>
            </div>
          ))
        )}
      </div>
    </>
  );
}

function formatTimestamp(timestamp: string): string {
  const match = timestamp.match(/\d{2}:\d{2}:\d{2}(?:\.\d+)?/);
  return match?.[0] ?? "";
}

function logEntrySearchText(entry: SimulatorLogEntry): string {
  return [
    entry.process,
    entry.message,
    entry.subsystem,
    entry.category,
    entry.level,
  ]
    .join(" ")
    .toLowerCase();
}

function collectAccessibilityPids(roots: AccessibilityNode[]): Set<number> {
  const pids = new Set<number>();
  const visit = (node: AccessibilityNode) => {
    if (typeof node.pid === "number" && Number.isFinite(node.pid)) {
      pids.add(node.pid);
    }
    for (const child of node.children ?? []) {
      visit(child);
    }
  };
  for (const root of roots) {
    visit(root);
  }
  return pids;
}

function collectActiveProcesses(
  entries: SimulatorLogEntry[],
  activePids: Set<number>,
): Set<string> {
  const processes = new Set<string>();
  for (const entry of entries) {
    if (entry.pid != null && activePids.has(entry.pid)) {
      processes.add(entry.process);
    }
  }
  return processes;
}

function isNearBottom(scroller: HTMLDivElement | null): boolean {
  if (!scroller) {
    return true;
  }

  return scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 8;
}

function serviceColor(entry: SimulatorLogEntry): string {
  const key = entry.process || entry.subsystem || "log";
  let hash = 0;
  for (let index = 0; index < key.length; index += 1) {
    hash = (hash * 31 + key.charCodeAt(index)) % 360;
  }

  if (entry.level.toLowerCase().includes("error")) {
    return "var(--error)";
  }
  if (entry.level.toLowerCase().includes("fault")) {
    return "var(--error)";
  }
  return `hsl(${hash} 72% 62%)`;
}

const LOG_LEVELS = ["error", "default", "info", "debug"] as const;

type LogLevelGroup = (typeof LOG_LEVELS)[number];

function logLevelGroup(level: string): LogLevelGroup {
  const normalized = level.toLowerCase();
  if (normalized.includes("error") || normalized.includes("fault")) {
    return "error";
  }
  if (normalized.includes("debug")) {
    return "debug";
  }
  if (normalized.includes("info")) {
    return "info";
  }
  return "default";
}

function mergeLogEntries(
  current: SimulatorLogEntry[],
  next: SimulatorLogEntry[],
): SimulatorLogEntry[] {
  const byKey = new Map<string, SimulatorLogEntry>();
  for (const entry of [...current, ...next]) {
    byKey.set(logEntryKey(entry), entry);
  }
  return [...byKey.values()]
    .sort((left, right) => left.timestamp.localeCompare(right.timestamp))
    .slice(-1000);
}

function logEntryKey(entry: SimulatorLogEntry): string {
  return [
    entry.timestamp,
    entry.pid ?? "",
    entry.process,
    entry.subsystem,
    entry.category,
    entry.message,
  ].join("\u0000");
}

function readStoredEntries(udid: string): SimulatorLogEntry[] {
  if (typeof window === "undefined") {
    return [];
  }

  try {
    const parsed = JSON.parse(
      window.sessionStorage.getItem(consoleStorageKey(udid)) ?? "[]",
    );
    return Array.isArray(parsed) ? parsed.slice(-1000) : [];
  } catch {
    return [];
  }
}

function writeStoredEntries(udid: string, entries: SimulatorLogEntry[]) {
  if (typeof window === "undefined") {
    return;
  }

  try {
    window.sessionStorage.setItem(
      consoleStorageKey(udid),
      JSON.stringify(entries.slice(-1000)),
    );
  } catch {
    // Session storage is a best-effort guard against browser focus reloads.
  }
}

function consoleStorageKey(udid: string): string {
  return `xcw-console-entries:${udid}`;
}
