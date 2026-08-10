export type EndpointKind = "api" | "websocket";

const LOCAL_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]", "::1"]);

function isLocalhost(url: URL): boolean {
  return LOCAL_HOSTS.has(url.hostname);
}

export function assertSecureEndpoint(
  rawUrl: string,
  kind: EndpointKind,
  developmentBuild: boolean,
): string {
  let url: URL;
  try {
    url = new URL(rawUrl);
  } catch {
    throw new Error(`Invalid ${kind} endpoint`);
  }

  const secureProtocol = kind === "api" ? "https:" : "wss:";
  const localProtocol = kind === "api" ? "http:" : "ws:";
  if (url.protocol === secureProtocol) {
    return url.toString().replace(/\/$/, "");
  }
  if (developmentBuild && url.protocol === localProtocol && isLocalhost(url)) {
    return url.toString().replace(/\/$/, "");
  }
  throw new Error(
    `${kind === "api" ? "HTTPS" : "WSS"} is required outside localhost development`,
  );
}

export function validateEndpointPair(
  apiUrl: string,
  wsUrl: string,
  developmentBuild: boolean,
): { apiUrl: string; wsUrl: string } {
  return {
    apiUrl: assertSecureEndpoint(apiUrl, "api", developmentBuild),
    wsUrl: assertSecureEndpoint(wsUrl, "websocket", developmentBuild),
  };
}
