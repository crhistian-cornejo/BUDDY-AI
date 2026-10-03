import { env, exports } from "cloudflare:workers";
import { expect, vi } from "vitest";

export const BASE = "https://relay.test";
export const OWNER = "test-owner-key";

export type Role = "desktop" | "phone";

export interface RoomInfo {
  room: string;
  key: string;
}

export function request(path: string, init: RequestInit = {}, bearer?: string): Promise<Response> {
  const headers = new Headers(init.headers);
  if (bearer !== undefined) headers.set("Authorization", `Bearer ${bearer}`);
  return exports.default.fetch(new Request(BASE + path, { ...init, headers }));
}

export async function createRoom(): Promise<RoomInfo> {
  const res = await request("/rooms", { method: "POST" }, OWNER);
  expect(res.status).toBe(201);
  return (await res.json()) as RoomInfo;
}

export function upgrade(room: string, role: string, key?: string): Promise<Response> {
  return request(`/rooms/${room}/ws?role=${role}`, { headers: { Upgrade: "websocket" } }, key);
}

// A test client: queues what arrives so tests can read it in order.
export class Client {
  private queue: (string | ArrayBuffer)[] = [];
  private waiters: ((data: string | ArrayBuffer) => void)[] = [];
  readonly closed: Promise<{ code: number; reason: string }>;

  constructor(private ws: WebSocket) {
    ws.accept();
    ws.addEventListener("message", (event) => {
      const data = event.data as string | ArrayBuffer;
      const waiter = this.waiters.shift();
      if (waiter) waiter(data);
      else this.queue.push(data);
    });
    this.closed = new Promise((resolve) => {
      ws.addEventListener("close", (event) => resolve({ code: event.code, reason: event.reason }));
    });
  }

  // The next frame as raw text; fails after two seconds of silence.
  nextText(): Promise<string> {
    const queued = this.queue.shift();
    if (queued !== undefined) return Promise.resolve(queued as string);
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error("no frame arrived")), 2000);
      this.waiters.push((data) => {
        clearTimeout(timer);
        resolve(data as string);
      });
    });
  }

  async next(): Promise<unknown> {
    return JSON.parse(await this.nextText());
  }

  send(frame: unknown): void {
    if (typeof frame === "string" || frame instanceof ArrayBuffer) this.ws.send(frame);
    else this.ws.send(JSON.stringify(frame));
  }

  close(): void {
    this.ws.close(1000, "bye");
  }
}

export async function connect(info: RoomInfo, role: Role): Promise<Client> {
  const res = await upgrade(info.room, role, info.key);
  expect(res.status).toBe(101);
  return new Client(res.webSocket!);
}

// A room with both roles connected and the presence frames already read.
export async function pairUp(): Promise<{ info: RoomInfo; desktop: Client; phone: Client }> {
  const info = await createRoom();
  const desktop = await connect(info, "desktop");
  expect(await desktop.next()).toEqual({ t: "peer", on: false });
  const phone = await connect(info, "phone");
  expect(await phone.next()).toEqual({ t: "peer", on: true });
  expect(await desktop.next()).toEqual({ t: "peer", on: true });
  return { info, desktop, phone };
}

export function stub(info: RoomInfo) {
  return env.ROOMS.get(env.ROOMS.idFromName(info.room));
}

export async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function base64(bytes: ArrayBuffer): string {
  return btoa(String.fromCharCode(...new Uint8Array(bytes)));
}

export function base64UrlDecode(text: string): Uint8Array {
  const padded = text.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(text.length / 4) * 4, "=");
  return Uint8Array.from(atob(padded), (c) => c.charCodeAt(0));
}

export interface TestKey {
  pem: string;
  publicKey: CryptoKey;
}

// A throwaway P-256 key in the same PEM form as Apple's .p8 file.
export async function throwawayKey(): Promise<TestKey> {
  const pair = (await crypto.subtle.generateKey({ name: "ECDSA", namedCurve: "P-256" }, true, [
    "sign",
    "verify",
  ])) as CryptoKeyPair;
  const der = (await crypto.subtle.exportKey("pkcs8", pair.privateKey)) as ArrayBuffer;
  const lines = base64(der).match(/.{1,64}/g)!.join("\n");
  return { pem: `-----BEGIN PRIVATE KEY-----\n${lines}\n-----END PRIVATE KEY-----\n`, publicKey: pair.publicKey };
}

// Checks an APNs provider token (ES256 JWT) and returns its header and claims.
export async function readJwt(
  jwt: string,
  publicKey: CryptoKey,
): Promise<{ header: Record<string, unknown>; claims: Record<string, unknown> }> {
  const [header, claims, signature] = jwt.split(".") as [string, string, string];
  const valid = await crypto.subtle.verify(
    { name: "ECDSA", hash: "SHA-256" },
    publicKey,
    base64UrlDecode(signature),
    new TextEncoder().encode(`${header}.${claims}`),
  );
  expect(valid).toBe(true);
  const decode = (part: string) => JSON.parse(new TextDecoder().decode(base64UrlDecode(part)));
  return { header: decode(header), claims: decode(claims) };
}

export const APNS = { keyId: "KEYID12345", teamId: "TEAMID6789", topic: "app.buddy.ios" };

// Sets the APNs secrets the way `wrangler secret put` would.
export function setApns(pem: string): void {
  Object.assign(env, {
    APNS_KEY_P8: pem,
    APNS_KEY_ID: APNS.keyId,
    APNS_TEAM_ID: APNS.teamId,
    APNS_TOPIC: APNS.topic,
  });
}

export function clearApns(): void {
  const secrets = env as Partial<Cloudflare.Env>;
  delete secrets.APNS_KEY_P8;
  delete secrets.APNS_KEY_ID;
  delete secrets.APNS_TEAM_ID;
  delete secrets.APNS_TOPIC;
}

export interface Call {
  url: string;
  method: string;
  headers: Headers;
  body: string;
}

// Replaces outbound fetch; records each request and answers with `reply`.
// The pieces are copied out because a body made inside the room cannot be
// read from the test.
export function mockApns(reply: () => Response = () => new Response(null, { status: 200 })): Call[] {
  const calls: Call[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    calls.push({
      url: String(input),
      method: init?.method ?? "GET",
      headers: new Headers(init?.headers),
      body: String(init?.body),
    });
    return reply();
  });
  return calls;
}
