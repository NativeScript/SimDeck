import { apiRequest } from "./client";
import type {
  ChromeProfile,
  SimulatorMetadata,
  SimulatorsResponse,
} from "./types";

export async function listSimulators(): Promise<SimulatorMetadata[]> {
  const data = await apiRequest<SimulatorsResponse>("/api/simulators");
  return data.simulators ?? [];
}

export async function fetchChromeProfile(udid: string): Promise<ChromeProfile> {
  return apiRequest<ChromeProfile>(`/api/simulators/${udid}/chrome-profile`);
}
