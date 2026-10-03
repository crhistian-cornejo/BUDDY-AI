import { runInDurableObject } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import {
  APNS,
  type Client,
  clearApns,
  connect,
  mockApns,
  pairUp,
  readJwt,
  type RoomInfo,
  setApns,
  stub,
  type TestKey,
  throwawayKey,
} from "./helpers";

const ALERT = "a1".repeat(32);
const LIVE = "b2".repeat(80);

let key: TestKey;

beforeAll(async () => {
  key = await throwawayKey();
});

afterEach(() => {
  vi.restoreAllMocks();
  clearApns();
});

// A paired room whose phone has registered its tokens.
async function ready(sandbox = false, live: string | null = LIVE) {
  const room = await pairUp();
  room.phone.send({ t: "token", alert: ALERT, live, sandbox });
  await settled(room.phone, room.desktop);
  return room;
}

// Waits until the relay has handled everything `from` sent so far.
async function settled(from: Client, to: Client): Promise<void> {
  from.send({ t: "msg", d: "sync" });
  expect(await to.next()).toEqual({ t: "msg", d: "sync" });
}

const tokens = (info: RoomInfo) => runInDurableObject(stub(info), (_room, state) => state.storage.get("tokens"));

describe("push without APNs secrets", () => {
  it("answers no-apns and calls nobody", async () => {
    const calls = mockApns();
    const { desktop, phone } = await ready();
    desktop.send({ t: "push", kind: "alert", d: "c2VhbGVk", urgent: false });
    expect(await desktop.next()).toEqual({ t: "error", code: "no-apns" });
    await settled(desktop, phone);
    expect(calls).toHaveLength(0);
  });

  it("needs all four secrets", async () => {
    setApns(key.pem);
    delete (env as Partial<Cloudflare.Env>).APNS_TOPIC;
    const calls = mockApns();
    const { desktop } = await ready();
    desktop.send({ t: "push", kind: "alert", d: "c2VhbGVk", urgent: false });
    expect(await desktop.next()).toEqual({ t: "error", code: "no-apns" });
    expect(calls).toHaveLength(0);
  });
});

describe("push with APNs secrets", () => {
  it("delivers an alert to Apple with the expected request", async () => {
    setApns(key.pem);
    const calls = mockApns();
    const { info, desktop, phone } = await ready();
    const before = Math.floor(Date.now() / 1000);

    desktop.send({ t: "push", kind: "alert", d: "c2VhbGVk", urgent: false });
    await settled(desktop, phone);

    expect(calls).toHaveLength(1);
    const call = calls[0]!;
    expect(call.method).toBe("POST");
    expect(call.url).toBe(`https://api.push.apple.com/3/device/${ALERT}`);
    expect(call.headers.get("apns-topic")).toBe(APNS.topic);
    expect(call.headers.get("apns-push-type")).toBe("alert");
    expect(call.headers.get("apns-priority")).toBe("10");
    expect(call.headers.get("apns-collapse-id")).toBeNull();
    expect(JSON.parse(call.body)).toEqual({
      aps: {
        alert: { title: "Buddy", body: "Novedad en tu equipo" },
        "mutable-content": 1,
        sound: "default",
        "interruption-level": "active",
        "thread-id": info.room,
      },
      d: "c2VhbGVk",
    });

    const authorization = call.headers.get("authorization")!;
    expect(authorization.startsWith("bearer ")).toBe(true);
    const { header, claims } = await readJwt(authorization.slice("bearer ".length), key.publicKey);
    expect(header).toEqual({ alg: "ES256", kid: APNS.keyId });
    expect(Object.keys(claims).sort()).toEqual(["iat", "iss"]);
    expect(claims.iss).toBe(APNS.teamId);
    expect(claims.iat).toBeGreaterThanOrEqual(before);
    expect(claims.iat).toBeLessThanOrEqual(Math.floor(Date.now() / 1000));
  });

  it("marks urgent alerts as time-sensitive and passes the collapse id", async () => {
    setApns(key.pem);
    const calls = mockApns();
    const { desktop, phone } = await ready();

    desktop.send({ t: "push", kind: "alert", d: "eA==", urgent: true, collapse: "chat-7" });
    await settled(desktop, phone);

    const call = calls[0]!;
    expect(call.headers.get("apns-collapse-id")).toBe("chat-7");
    const body = JSON.parse(call.body) as { aps: Record<string, unknown> };
    expect(body.aps["interruption-level"]).toBe("time-sensitive");
  });

  it("uses Apple's sandbox host when the phone said so", async () => {
    setApns(key.pem);
    const calls = mockApns();
    const { desktop, phone } = await ready(true);

    desktop.send({ t: "push", kind: "alert", d: "eA==", urgent: false });
    desktop.send({ t: "push", kind: "live", d: "{}", urgent: false });
    await settled(desktop, phone);

    expect(calls.map((call) => call.url)).toEqual([
      `https://api.sandbox.push.apple.com/3/device/${ALERT}`,
      `https://api.sandbox.push.apple.com/3/device/${LIVE}`,
    ]);
  });

  it("delivers a live activity update with the live token", async () => {
    setApns(key.pem);
    const calls = mockApns();
    const { desktop, phone } = await ready();
    const before = Math.floor(Date.now() / 1000);

    desktop.send({ t: "push", kind: "live", d: '{"pose":3,"state":2}', urgent: false });
    await settled(desktop, phone);

    expect(calls).toHaveLength(1);
    const call = calls[0]!;
    expect(call.method).toBe("POST");
    expect(call.url).toBe(`https://api.push.apple.com/3/device/${LIVE}`);
    expect(call.headers.get("apns-topic")).toBe(`${APNS.topic}.push-type.liveactivity`);
    expect(call.headers.get("apns-push-type")).toBe("liveactivity");
    await readJwt(call.headers.get("authorization")!.slice("bearer ".length), key.publicKey);
    const body = JSON.parse(call.body) as { aps: { timestamp: number } };
    expect(body).toEqual({
      aps: { timestamp: body.aps.timestamp, event: "update", "content-state": { pose: 3, state: 2 } },
    });
    expect(body.aps.timestamp).toBeGreaterThanOrEqual(before);
    expect(body.aps.timestamp).toBeLessThanOrEqual(Math.floor(Date.now() / 1000));
  });

  it("signs once and reuses the provider token", async () => {
    setApns(key.pem);
    const calls = mockApns();
    const { desktop, phone } = await ready();

    desktop.send({ t: "push", kind: "alert", d: "MQ==", urgent: false });
    desktop.send({ t: "push", kind: "alert", d: "Mg==", urgent: false });
    await settled(desktop, phone);

    expect(calls).toHaveLength(2);
    expect(calls[1]!.headers.get("authorization")).toBe(calls[0]!.headers.get("authorization"));
  });

  it("drops the push silently when no token is stored", async () => {
    setApns(key.pem);
    const calls = mockApns();
    const { desktop, phone } = await pairUp();

    desktop.send({ t: "push", kind: "alert", d: "eA==", urgent: false });
    desktop.send({ t: "push", kind: "live", d: "{}", urgent: false });
    // The next thing the desktop hears is the phone, not an error.
    await settled(desktop, phone);
    await settled(phone, desktop);
    expect(calls).toHaveLength(0);
  });

  it("drops a live update silently when only the alert token is stored", async () => {
    setApns(key.pem);
    const calls = mockApns();
    const { desktop, phone } = await ready(false, null);

    desktop.send({ t: "push", kind: "live", d: "{}", urgent: false });
    await settled(desktop, phone);
    await settled(phone, desktop);
    expect(calls).toHaveLength(0);
  });

  it("forgets a token when Apple answers 410", async () => {
    setApns(key.pem);
    const calls = mockApns(() => Response.json({ reason: "Unregistered" }, { status: 410 }));
    const { info, desktop, phone } = await ready();

    desktop.send({ t: "push", kind: "alert", d: "eA==", urgent: false });
    await settled(desktop, phone);
    expect(calls).toHaveLength(1);
    expect(await tokens(info)).toEqual({ alert: null, live: LIVE, sandbox: false });

    desktop.send({ t: "push", kind: "alert", d: "eA==", urgent: false });
    await settled(desktop, phone);
    expect(calls).toHaveLength(1);
  });

  it("forgets a token when Apple answers 400 BadDeviceToken or Unregistered", async () => {
    setApns(key.pem);
    for (const reason of ["BadDeviceToken", "Unregistered"]) {
      mockApns(() => Response.json({ reason }, { status: 400 }));
      const { info, desktop, phone } = await ready();
      desktop.send({ t: "push", kind: "live", d: "{}", urgent: false });
      await settled(desktop, phone);
      expect(await tokens(info), reason).toEqual({ alert: ALERT, live: null, sandbox: false });
      vi.restoreAllMocks();
    }
  });

  it("keeps the token on other Apple errors", async () => {
    setApns(key.pem);
    const replies = [
      Response.json({ reason: "PayloadEmpty" }, { status: 400 }),
      new Response("not json", { status: 400 }),
      Response.json({ reason: "TooManyRequests" }, { status: 429 }),
      Response.json({ reason: "InternalServerError" }, { status: 500 }),
    ];
    const calls = mockApns(() => replies.shift()!);
    const { info, desktop, phone } = await ready();
    for (let i = 0; i < 4; i++) desktop.send({ t: "push", kind: "alert", d: "eA==", urgent: false });
    await settled(desktop, phone);
    expect(calls).toHaveLength(4);
    expect(await tokens(info)).toEqual({ alert: ALERT, live: LIVE, sandbox: false });
  });

  it("survives a network failure towards Apple", async () => {
    setApns(key.pem);
    vi.spyOn(globalThis, "fetch").mockRejectedValue(new Error("network down"));
    const { info, desktop, phone } = await ready();
    desktop.send({ t: "push", kind: "alert", d: "eA==", urgent: false });
    await settled(desktop, phone);
    expect(await tokens(info)).toEqual({ alert: ALERT, live: LIVE, sandbox: false });
  });

  it("allows 60 pushes per hour per room, then answers rate", async () => {
    setApns(key.pem);
    const calls = mockApns();
    const { desktop, phone } = await ready();

    for (let i = 0; i < 61; i++) desktop.send({ t: "push", kind: "alert", d: "eA==", urgent: false });
    expect(await desktop.next()).toEqual({ t: "error", code: "rate" });
    await settled(desktop, phone);
    expect(calls).toHaveLength(60);
  });

  it("keeps the hourly count when the desktop reconnects", async () => {
    setApns(key.pem);
    const calls = mockApns();
    const { info, desktop, phone } = await ready();

    for (let i = 0; i < 60; i++) desktop.send({ t: "push", kind: "alert", d: "eA==", urgent: false });
    await settled(desktop, phone);

    const newer = await connect(info, "desktop");
    expect(await newer.next()).toEqual({ t: "peer", on: true });
    newer.send({ t: "push", kind: "alert", d: "eA==", urgent: false });
    expect(await newer.next()).toEqual({ t: "error", code: "rate" });
    expect(calls).toHaveLength(60);
  });
});

describe("push frames that are refused", () => {
  it("refuses push from the phone", async () => {
    setApns(key.pem);
    const calls = mockApns();
    const { desktop, phone } = await ready();
    phone.send({ t: "push", kind: "alert", d: "eA==", urgent: true });
    expect(await phone.next()).toEqual({ t: "error", code: "bad-frame" });
    await settled(phone, desktop);
    expect(calls).toHaveLength(0);
  });

  it("refuses malformed push frames", async () => {
    setApns(key.pem);
    const calls = mockApns();
    const { desktop, phone } = await ready();
    const bad = [
      { t: "push", kind: "banner", d: "eA==", urgent: false },
      { t: "push", kind: "alert", d: 7, urgent: false },
      { t: "push", kind: "alert", d: "eA==", urgent: "yes" },
      { t: "push", kind: "alert", d: "eA==", urgent: false, collapse: 7 },
      { t: "push", kind: "alert", d: "eA==", urgent: false, collapse: "x".repeat(65) },
      { t: "push", kind: "live", d: "not json", urgent: false },
    ];
    for (const frame of bad) {
      desktop.send(frame);
      expect(await desktop.next(), JSON.stringify(frame)).toEqual({ t: "error", code: "bad-frame" });
    }
    await settled(desktop, phone);
    expect(calls).toHaveLength(0);
  });
});
