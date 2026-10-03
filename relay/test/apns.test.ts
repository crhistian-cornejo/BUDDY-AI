import { beforeAll, describe, expect, it } from "vitest";
import { apnsConfig, providerToken } from "../src/apns";
import { readJwt, type TestKey, throwawayKey } from "./helpers";

let key: TestKey;

beforeAll(async () => {
  key = await throwawayKey();
});

const MINUTE = 60_000;

describe("apnsConfig", () => {
  const full = { APNS_KEY_P8: "pem", APNS_KEY_ID: "kid", APNS_TEAM_ID: "team", APNS_TOPIC: "topic" };

  it("reads the four secrets", () => {
    expect(apnsConfig(full as Cloudflare.Env)).toEqual({ key: "pem", keyId: "kid", teamId: "team", topic: "topic" });
  });

  it("is null when any secret is missing or empty", () => {
    for (const name of Object.keys(full)) {
      expect(apnsConfig({ ...full, [name]: undefined } as Cloudflare.Env), name).toBeNull();
      expect(apnsConfig({ ...full, [name]: "" } as Cloudflare.Env), name).toBeNull();
    }
  });
});

describe("providerToken", () => {
  it("signs an ES256 token with the key id and the team", async () => {
    const config = { key: key.pem, keyId: "KID0000001", teamId: "TEAM000001", topic: "t" };
    const now = 1_800_000_000_000;
    const { header, claims } = await readJwt(await providerToken(config, now), key.publicKey);
    expect(header).toEqual({ alg: "ES256", kid: "KID0000001" });
    expect(claims).toEqual({ iss: "TEAM000001", iat: 1_800_000_000 });
  });

  it("reuses the token for 40 minutes, then signs a new one", async () => {
    const config = { key: key.pem, keyId: "KID0000002", teamId: "TEAM000002", topic: "t" };
    const start = 1_800_000_000_000;
    const first = await providerToken(config, start);
    expect(await providerToken(config, start + 39 * MINUTE)).toBe(first);

    const later = await providerToken(config, start + 41 * MINUTE);
    expect(later).not.toBe(first);
    const { claims } = await readJwt(later, key.publicKey);
    expect(claims.iat).toBe(1_800_000_000 + 41 * 60);
  });

  it("does not reuse a token across different keys", async () => {
    const other = await throwawayKey();
    const now = 1_800_000_000_000;
    const first = await providerToken({ key: key.pem, keyId: "KID0000003", teamId: "TEAM", topic: "t" }, now);
    const second = await providerToken({ key: other.pem, keyId: "KID0000003", teamId: "TEAM", topic: "t" }, now);
    await readJwt(first, key.publicKey);
    await readJwt(second, other.publicKey);
  });

  it("accepts a key pasted with escaped line breaks", async () => {
    const config = { key: key.pem.replace(/\n/g, "\\n"), keyId: "KID0000004", teamId: "TEAM", topic: "t" };
    await readJwt(await providerToken(config, 1_800_000_000_000), key.publicKey);
  });
});
