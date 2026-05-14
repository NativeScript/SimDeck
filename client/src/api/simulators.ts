import { apiRequest } from "./client";
import type {
  AccessibilitySourcePreference,
  AccessibilityTreeResponse,
  ChromeDevToolsTargetDiscovery,
  ChromeProfile,
  CreateSimulatorRequest,
  CreateSimulatorResponse,
  InspectorRequestResponse,
  SimulatorPerformanceResponse,
  SimulatorLogsResponse,
  SimulatorCreateOptionsResponse,
  SimulatorMetadata,
  SimulatorProcessListResponse,
  SimulatorStateResponse,
  SimulatorsResponse,
  StackSampleResponse,
  WebKitTargetDiscovery,
} from "./types";

export async function listSimulators(
  options: RequestInit = {},
): Promise<SimulatorMetadata[]> {
  const data = await apiRequest<SimulatorsResponse>("/api/simulators", options);
  return data.simulators ?? [];
}

export async function fetchSimulatorCreateOptions(
  options: RequestInit = {},
): Promise<SimulatorCreateOptionsResponse> {
  return apiRequest<SimulatorCreateOptionsResponse>(
    "/api/simulators/create-options",
    options,
  );
}

export async function createSimulator(
  payload: CreateSimulatorRequest,
): Promise<CreateSimulatorResponse> {
  return apiRequest<CreateSimulatorResponse>("/api/simulators", {
    body: JSON.stringify(payload),
    method: "POST",
  });
}

export async function fetchChromeProfile(udid: string): Promise<ChromeProfile> {
  return apiRequest<ChromeProfile>(`/api/simulators/${udid}/chrome-profile`);
}

export async function fetchSimulatorState(
  udid: string,
  options: RequestInit = {},
): Promise<SimulatorStateResponse> {
  return apiRequest<SimulatorStateResponse>(
    `/api/simulators/${encodeURIComponent(udid)}/state`,
    options,
  );
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

export async function fetchAccessibilityPoint(
  udid: string,
  x: number,
  y: number,
  options: RequestInit & { maxDepth?: number } = {},
): Promise<AccessibilityTreeResponse> {
  const { maxDepth, ...requestOptions } = options;
  const params = new URLSearchParams({
    x: String(x),
    y: String(y),
  });
  if (maxDepth != null) {
    params.set("maxDepth", String(maxDepth));
  }
  return apiRequest<AccessibilityTreeResponse>(
    `/api/simulators/${encodeURIComponent(udid)}/accessibility-point?${params}`,
    requestOptions,
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

export async function fetchSimulatorProcesses(
  udid: string,
  options: RequestInit = {},
): Promise<SimulatorProcessListResponse> {
  return apiRequest<SimulatorProcessListResponse>(
    `/api/simulators/${encodeURIComponent(udid)}/processes`,
    options,
  );
}

export async function fetchSimulatorPerformance(
  udid: string,
  options: {
    pid?: number | null;
    windowMs?: number;
    request?: RequestInit;
  } = {},
): Promise<SimulatorPerformanceResponse> {
  const params = new URLSearchParams();
  if (options.pid != null) {
    params.set("pid", String(options.pid));
  }
  if (options.windowMs != null) {
    params.set("windowMs", String(options.windowMs));
  }
  const query = params.size > 0 ? `?${params}` : "";
  return apiRequest<SimulatorPerformanceResponse>(
    `/api/simulators/${encodeURIComponent(udid)}/performance${query}`,
    options.request ?? {},
  );
}

export async function sampleSimulatorProcess(
  udid: string,
  pid: number,
  seconds = 3,
): Promise<StackSampleResponse> {
  const params = new URLSearchParams({ seconds: String(seconds) });
  return apiRequest<StackSampleResponse>(
    `/api/simulators/${encodeURIComponent(udid)}/processes/${pid}/sample?${params}`,
    { method: "POST" },
  );
}

export async function fetchWebKitTargets(
  udid: string,
  options: RequestInit = {},
): Promise<WebKitTargetDiscovery> {
  return apiRequest<WebKitTargetDiscovery>(
    `/api/simulators/${encodeURIComponent(udid)}/webkit/targets`,
    options,
  );
}

export async function fetchChromeDevToolsTargets(
  udid: string,
  options: RequestInit = {},
): Promise<ChromeDevToolsTargetDiscovery> {
  return apiRequest<ChromeDevToolsTargetDiscovery>(
    `/api/simulators/${encodeURIComponent(udid)}/devtools/targets`,
    options,
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
