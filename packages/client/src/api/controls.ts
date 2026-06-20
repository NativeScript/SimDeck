import { accessTokenFromLocation, apiHeaders, apiRequest } from "./client";
import { apiUrl } from "./config";
import type {
  ButtonPayload,
  CrownPayload,
  EdgeTouchPayload,
  InstallUploadResponse,
  KeyPayload,
  LaunchPayload,
  MultiTouchPayload,
  OpenUrlPayload,
  ScrollPayload,
  SimulatorMetadata,
  SimulatorResponse,
  TouchPayload,
} from "./types";

export type ControlMessage =
  | ({ type: "touch" } & TouchPayload)
  | ({ type: "edgeTouch" } & EdgeTouchPayload)
  | ({ type: "multiTouch" } & MultiTouchPayload)
  | ({ type: "key" } & KeyPayload)
  | ({ type: "button" } & ButtonPayload)
  | ({ type: "crown" } & CrownPayload)
  | ({ type: "scroll" } & ScrollPayload)
  | { type: "dismissKeyboard" }
  | { type: "toggleSoftwareKeyboard" }
  | { type: "home" }
  | { type: "appSwitcher" }
  | { type: "rotateLeft" }
  | { type: "rotateRight" }
  | { type: "toggleAppearance" };

export interface ScreenRecordingStartResponse {
  ok: boolean;
  recordingId: string;
}

async function postSimulatorAction(
  udid: string,
  action: string,
  payload?: LaunchPayload | OpenUrlPayload,
): Promise<SimulatorMetadata | null> {
  if (action === "launch" || action === "open-url") {
    const response = await apiRequest<{
      ok: boolean;
      simulator?: SimulatorMetadata | null;
    }>(`/api/simulators/${encodeURIComponent(udid)}/action`, {
      method: "POST",
      body: JSON.stringify({
        action: action === "open-url" ? "openUrl" : "launch",
        ...payload,
      }),
    });
    return response.simulator ?? null;
  }
  const response = await apiRequest<SimulatorResponse | { ok: boolean }>(
    `/api/simulators/${udid}/${action}`,
    {
      method: "POST",
      body: payload ? JSON.stringify(payload) : undefined,
    },
  );

  return "simulator" in response ? response.simulator : null;
}

export function bootSimulator(udid: string) {
  return postSimulatorAction(udid, "boot");
}

export function shutdownSimulator(udid: string) {
  return postSimulatorAction(udid, "shutdown");
}

export function openSimulatorUrl(udid: string, payload: OpenUrlPayload) {
  return postSimulatorAction(udid, "open-url", payload);
}

export function launchSimulatorBundle(udid: string, payload: LaunchPayload) {
  return postSimulatorAction(udid, "launch", payload);
}

export function toggleSimulatorAppearance(udid: string) {
  return apiRequest<{ ok: boolean }>(
    `/api/simulators/${encodeURIComponent(udid)}/action`,
    {
      method: "POST",
      body: JSON.stringify({ action: "toggleAppearance" }),
    },
  );
}

export function uploadSimulatorApp(
  udid: string,
  file: File,
): Promise<InstallUploadResponse> {
  return apiRequest<InstallUploadResponse>(
    `/api/simulators/${encodeURIComponent(udid)}/install-upload`,
    {
      body: file,
      headers: {
        "Content-Type": "application/octet-stream",
        "X-SimDeck-Filename": encodeURIComponent(file.name || "app-upload"),
      },
      method: "POST",
    },
  );
}

export function simulatorControlSocketUrl(udid: string) {
  const url = new URL(
    apiUrl(`/api/simulators/${encodeURIComponent(udid)}/control`),
    window.location.href,
  );
  const token = accessTokenFromLocation();
  if (token) {
    url.searchParams.set("simdeckToken", token);
  }
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.toString();
}

async function fetchSimulatorBlob(
  path: string,
  options: RequestInit = {},
): Promise<Blob> {
  const { headers, ...rest } = options;
  const response = await fetch(apiUrl(path), {
    ...rest,
    headers: apiHeaders(headers),
  });
  if (!response.ok) {
    const contentType = response.headers.get("content-type") ?? "";
    if (contentType.includes("application/json")) {
      const body = (await response.json()) as { error?: string };
      throw new Error(
        body.error ?? `Request failed with status ${response.status}`,
      );
    }
    throw new Error(
      (await response.text()) ||
        `Request failed with status ${response.status}`,
    );
  }
  return response.blob();
}

export function captureSimulatorScreenshot(
  udid: string,
  options: { withBezel?: boolean } = {},
): Promise<Blob> {
  const params = options.withBezel ? "?bezel=true" : "";
  return fetchSimulatorBlob(
    `/api/simulators/${encodeURIComponent(udid)}/screenshot.png${params}`,
  );
}

export function recordSimulatorScreen(
  udid: string,
  seconds = 5,
): Promise<Blob> {
  return fetchSimulatorBlob(
    `/api/simulators/${encodeURIComponent(udid)}/screen-recording`,
    {
      body: JSON.stringify({ seconds }),
      method: "POST",
    },
  );
}

export function startSimulatorScreenRecording(
  udid: string,
): Promise<ScreenRecordingStartResponse> {
  return apiRequest<ScreenRecordingStartResponse>(
    `/api/simulators/${encodeURIComponent(udid)}/screen-recording/start`,
    {
      method: "POST",
    },
  );
}

export function stopSimulatorScreenRecording(
  udid: string,
  recordingId: string,
): Promise<Blob> {
  return fetchSimulatorBlob(
    `/api/simulators/${encodeURIComponent(udid)}/screen-recording/${encodeURIComponent(recordingId)}/stop`,
    {
      method: "POST",
    },
  );
}
