import type {
  AccessibilitySource,
  AccessibilitySourcePreference,
} from "../api/types";
import type { Point, ViewMode } from "../features/viewport/types";

export interface PersistedViewportState {
  pan: Point;
  rotationQuarterTurns: number;
  viewMode: ViewMode;
  zoom: number | null;
}

export interface PersistedUiState {
  accessibilitySelectedByUDID?: Record<string, string>;
  bundleIDValue?: string;
  openURLValue?: string;
  search?: string;
  selectedUDID?: string;
  viewportByUDID?: Record<string, PersistedViewportState>;
}

export const UI_STATE_STORAGE_KEY = "xcw-ui-state";
export const DEBUG_VISIBLE_STORAGE_KEY = "xcw-debug-visible";
export const HIERARCHY_VISIBLE_STORAGE_KEY = "xcw-hierarchy-visible";
export const ACCESSIBILITY_SOURCE_STORAGE_KEY = "xcw-hierarchy-source";

const ACCESSIBILITY_SOURCE_ORDER: AccessibilitySource[] = [
  "nativescript",
  "in-app-inspector",
  "native-ax",
];

export const DEFAULT_VIEWPORT_STATE: PersistedViewportState = {
  pan: { x: 0, y: 0 },
  rotationQuarterTurns: 0,
  viewMode: "center",
  zoom: null,
};

export function readStoredFlag(storageKey: string): boolean {
  if (typeof window === "undefined") {
    return false;
  }

  return window.localStorage.getItem(storageKey) === "1";
}

export function writeStoredFlag(storageKey: string, value: boolean): void {
  if (typeof window === "undefined") {
    return;
  }

  window.localStorage.setItem(storageKey, value ? "1" : "0");
}

export function readPersistedUiState(): PersistedUiState {
  if (typeof window === "undefined") {
    return {};
  }

  try {
    const parsed = JSON.parse(
      window.localStorage.getItem(UI_STATE_STORAGE_KEY) ?? "{}",
    ) as PersistedUiState;
    return sanitizePersistedUiState(parsed);
  } catch {
    return {};
  }
}

export function readStoredAccessibilitySource(): AccessibilitySourcePreference {
  if (typeof window === "undefined") {
    return "auto";
  }

  const source = window.localStorage.getItem(ACCESSIBILITY_SOURCE_STORAGE_KEY);
  return source === "auto" || isAccessibilitySource(source) ? source : "auto";
}

export function sanitizeAccessibilitySources(
  value: unknown,
): AccessibilitySource[] {
  if (!Array.isArray(value)) {
    return [];
  }

  const sourceSet = new Set(value.filter(isAccessibilitySource));
  return ACCESSIBILITY_SOURCE_ORDER.filter((source) => sourceSet.has(source));
}

export function isAccessibilitySource(
  value: unknown,
): value is AccessibilitySource {
  return (
    value === "nativescript" ||
    value === "in-app-inspector" ||
    value === "native-ax"
  );
}

export function writePersistedUiState(
  updater: (current: PersistedUiState) => PersistedUiState,
): void {
  if (typeof window === "undefined") {
    return;
  }

  const nextState = sanitizePersistedUiState(updater(readPersistedUiState()));
  window.localStorage.setItem(UI_STATE_STORAGE_KEY, JSON.stringify(nextState));
}

export function viewportStateForUDID(
  state: PersistedUiState,
  udid: string,
): PersistedViewportState {
  return state.viewportByUDID?.[udid] ?? DEFAULT_VIEWPORT_STATE;
}

export function sanitizePersistedUiState(
  state: PersistedUiState,
): PersistedUiState {
  const viewportByUDID = Object.fromEntries(
    Object.entries(state.viewportByUDID ?? {})
      .map(([udid, viewport]) => [udid, sanitizeViewportState(viewport)])
      .filter((entry): entry is [string, PersistedViewportState] =>
        Boolean(entry[1]),
      ),
  );

  return {
    accessibilitySelectedByUDID: state.accessibilitySelectedByUDID ?? {},
    bundleIDValue: stringOrUndefined(state.bundleIDValue),
    openURLValue: stringOrUndefined(state.openURLValue),
    search: stringOrUndefined(state.search),
    selectedUDID: stringOrUndefined(state.selectedUDID),
    viewportByUDID,
  };
}

function sanitizeViewportState(
  state: PersistedViewportState | undefined,
): PersistedViewportState | null {
  if (!state) {
    return null;
  }

  return {
    pan: isPoint(state.pan) ? state.pan : DEFAULT_VIEWPORT_STATE.pan,
    rotationQuarterTurns: Number.isFinite(state.rotationQuarterTurns)
      ? state.rotationQuarterTurns
      : DEFAULT_VIEWPORT_STATE.rotationQuarterTurns,
    viewMode: isViewMode(state.viewMode)
      ? state.viewMode
      : DEFAULT_VIEWPORT_STATE.viewMode,
    zoom: state.zoom == null || Number.isFinite(state.zoom) ? state.zoom : null,
  };
}

function isPoint(value: unknown): value is Point {
  return Boolean(
    value &&
    typeof value === "object" &&
    Number.isFinite((value as Point).x) &&
    Number.isFinite((value as Point).y),
  );
}

function isViewMode(value: unknown): value is ViewMode {
  return value === "center" || value === "fit" || value === "manual";
}

function stringOrUndefined(value: unknown): string | undefined {
  return typeof value === "string" ? value : undefined;
}
