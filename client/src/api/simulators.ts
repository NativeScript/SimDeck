import { apiRequest } from "./client";
import type {
  AccessibilitySourcePreference,
  AccessibilityTreeResponse,
  ChromeProfile,
  InspectorRequestResponse,
  SimulatorLogsResponse,
  SimulatorMetadata,
  SimulatorsResponse,
} from "./types";

export async function listSimulators(
  options: RequestInit = {},
): Promise<SimulatorMetadata[]> {
  const data = await apiRequest<SimulatorsResponse>("/api/simulators", options);
  return data.simulators ?? [];
}

export async function fetchChromeProfile(udid: string): Promise<ChromeProfile> {
  return apiRequest<ChromeProfile>(`/api/simulators/${udid}/chrome-profile`);
}

export async function fetchAccessibilityTree(
  udid: string,
  source: AccessibilitySourcePreference = "auto",
  options: { maxDepth?: number } = {},
): Promise<AccessibilityTreeResponse> {
  const params = new URLSearchParams();
  if (source !== "auto") {
    params.set("source", source);
  }
  if (options.maxDepth != null) {
    params.set("maxDepth", String(options.maxDepth));
  }
  const query = params.size > 0 ? `?${params}` : "";
  return apiRequest<AccessibilityTreeResponse>(
    `/api/simulators/${udid}/accessibility-tree${query}`,
  );
}

export async function fetchSimulatorLogs(
  udid: string,
  options: {
    levels?: string[];
    limit?: number;
    processes?: string[];
    query?: string;
    seconds?: number;
    backfill?: boolean;
  } = {},
): Promise<SimulatorLogsResponse> {
  const params = new URLSearchParams();
  if (options.backfill) {
    params.set("backfill", "true");
  }
  if (options.seconds) {
    params.set("seconds", String(options.seconds));
  }
  if (options.limit) {
    params.set("limit", String(options.limit));
  }
  if (options.levels?.length) {
    params.set("levels", options.levels.join(","));
  }
  if (options.processes?.length) {
    params.set("processes", options.processes.join(","));
  }
  if (options.query?.trim()) {
    params.set("q", options.query.trim());
  }
  const query = params.size > 0 ? `?${params}` : "";
  return apiRequest<SimulatorLogsResponse>(
    `/api/simulators/${udid}/logs${query}`,
  );
}

export async function sendInspectorRequest<T = unknown>(
  udid: string,
  method: string,
  params?: unknown,
): Promise<InspectorRequestResponse<T>> {
  return apiRequest<InspectorRequestResponse<T>>(
    `/api/simulators/${udid}/inspector/request`,
    {
      body: JSON.stringify({ method, params }),
      method: "POST",
    },
  );
}
