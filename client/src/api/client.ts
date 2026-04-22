import { API_ROOT } from "../shared/constants";

export async function apiRequest<T>(
  path: string,
  options: RequestInit = {},
): Promise<T> {
  const response = await fetch(`${API_ROOT}${path}`, {
    headers: { "Content-Type": "application/json" },
    ...options,
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
