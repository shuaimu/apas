import Constants from "expo-constants";

import { validateEndpointPair } from "./endpointPolicy";

interface MobileExtra {
  apiUrl?: string;
  wsUrl?: string;
  buildProfile?: string;
}

const extra = (Constants.expoConfig?.extra ?? {}) as MobileExtra;
const developmentBuild = __DEV__ && extra.buildProfile !== "production";

export const endpoints = validateEndpointPair(
  extra.apiUrl ?? "https://apas.mpaxos.com",
  extra.wsUrl ?? "wss://apas.mpaxos.com",
  developmentBuild,
);

export const MOBILE_PROTOCOL_VERSION = 1;
export const MOBILE_CAPABILITIES = [
  "bootstrap",
  "code_events",
  "coding_mutations",
  "terminal",
  "pane_work_summary_v1",
] as const;
