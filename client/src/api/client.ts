import { API_ROOT } from "../shared/constants";

export function accessTokenFromLocation(): string {
  if (typeof window === "undefined") {
    return "";
  }
  return new URLSearchParams(window.location.search).get("simdeckToken") ?? "";
}

export function apiHeaders(headers: HeadersInit = {}): HeadersInit {
  const token = accessTokenFromLocation();
  return {
    "Content-Type": "application/json",
    ...(token ? { "X-SimDeck-Token": token } : {}),
    ...headers,
  };
}

export async function apiRequest<T>(
  path: string,
  options: RequestInit = {},
): Promise<T> {
  const { headers, ...rest } = options;
  const response = await fetch(`${API_ROOT}${path}`, {
    ...rest,
    headers: apiHeaders(headers),
  });

  const contentType = response.headers.get("content-type") ?? "";
  if (!response.ok) {
    let message = `Request failed with status ${response.status}`;
    if (contentType.includes("application/json")) {
      const body = (await response.json()) as { error?: string };
      message = body.error ?? message;
    } else {
      message = await response.text();
    }
    throw new Error(message);
  }

  if (response.status === 204) {
    return null as T;
  }

  if (contentType.includes("application/json")) {
    return (await response.json()) as T;
  }

  return (await response.text()) as T;
}
