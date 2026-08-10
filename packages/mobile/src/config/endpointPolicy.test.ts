import { assertSecureEndpoint, validateEndpointPair } from "./endpointPolicy";

describe("mobile endpoint policy", () => {
  it("accepts production HTTPS and WSS", () => {
    expect(
      validateEndpointPair("https://apas.mpaxos.com", "wss://apas.mpaxos.com", false),
    ).toEqual({
      apiUrl: "https://apas.mpaxos.com",
      wsUrl: "wss://apas.mpaxos.com",
    });
  });

  it.each([
    ["http://apas.mpaxos.com", "api"],
    ["ws://apas.mpaxos.com", "websocket"],
    ["http://localhost:3000", "api"],
    ["ws://127.0.0.1:8080", "websocket"],
  ] as const)("rejects %s in production", (url, kind) => {
    expect(() => assertSecureEndpoint(url, kind, false)).toThrow();
  });

  it("permits cleartext localhost only in a development build", () => {
    expect(assertSecureEndpoint("http://localhost:3000", "api", true)).toBe(
      "http://localhost:3000",
    );
    expect(() => assertSecureEndpoint("http://192.168.1.20", "api", true)).toThrow();
  });
});
