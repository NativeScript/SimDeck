export interface SimDeckClientConfig {
  apiRoot?: string;
}

let clientConfig: Required<SimDeckClientConfig> = {
  apiRoot: "",
};

export function configureSimDeckClient(config: SimDeckClientConfig): void {
  clientConfig = {
    ...clientConfig,
    ...config,
    apiRoot: normalizeRoot(config.apiRoot ?? clientConfig.apiRoot),
  };
}

export function apiRoot(): string {
  return clientConfig.apiRoot;
}

export function apiUrl(path: string): string {
  const root = apiRoot();
  if (!root) {
    return path;
  }
  return `${root}${path.startsWith("/") ? path : `/${path}`}`;
}

function normalizeRoot(root: string): string {
  return root.replace(/\/+$/, "");
}
