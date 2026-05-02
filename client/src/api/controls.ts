import { accessTokenFromLocation, apiRequest } from "./client";
import { apiUrl } from "./config";
import type {
  KeyPayload,
  LaunchPayload,
  OpenUrlPayload,
  SimulatorMetadata,
  SimulatorResponse,
  TouchPayload,
} from "./types";

export type ControlMessage =
  | ({ type: "touch" } & TouchPayload)
  | ({ type: "key" } & KeyPayload)
  | { type: "dismissKeyboard" }
  | { type: "home" }
  | { type: "appSwitcher" }
  | { type: "rotateLeft" }
  | { type: "rotateRight" }
  | { type: "toggleAppearance" };

async function postSimulatorAction(
  udid: string,
  action: string,
  payload?: KeyPayload | LaunchPayload | OpenUrlPayload | TouchPayload,
): Promise<SimulatorMetadata | null> {
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

export function toggleAppearance(udid: string) {
  return postSimulatorAction(udid, "toggle-appearance");
}

export function openSimulatorUrl(udid: string, payload: OpenUrlPayload) {
  return postSimulatorAction(udid, "open-url", payload);
}

export function launchSimulatorBundle(udid: string, payload: LaunchPayload) {
  return postSimulatorAction(udid, "launch", payload);
}

export function sendTouch(udid: string, payload: TouchPayload) {
  return postSimulatorAction(udid, "touch", payload);
}

export function sendKey(udid: string, payload: KeyPayload) {
  return postSimulatorAction(udid, "key", payload);
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

export function dismissKeyboard(udid: string) {
  return postSimulatorAction(udid, "dismiss-keyboard");
}

export function pressHome(udid: string) {
  return postSimulatorAction(udid, "home");
}

export function openAppSwitcher(udid: string) {
  return postSimulatorAction(udid, "app-switcher");
}

export function rotateLeft(udid: string) {
  return postSimulatorAction(udid, "rotate-left");
}

export function rotateRight(udid: string) {
  return postSimulatorAction(udid, "rotate-right");
}
