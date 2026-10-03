// One room: the meeting point of a desktop and its phone.
//
// The room forwards opaque frames between the two roles, reports presence and
// hands pushes to Apple. It stores the hash of its key, the phone's push
// tokens, two timestamps and the hourly push count. Frame contents are never
// stored or logged.

import { DurableObject } from "cloudflare:workers";
import { apnsConfig, deliver, type Push } from "./apns";

export type Role = "desktop" | "phone";
type ErrorCode = "too-big" | "rate" | "no-apns" | "bad-frame";

const MAX_FRAME_BYTES = 256 * 1024;
const FRAMES_PER_MINUTE = 600;
const PUSHES_PER_HOUR = 60;
const MINUTE_MS = 60_000;
const HOUR_MS = 60 * MINUTE_MS;
const MAX_COLLAPSE_BYTES = 64; // Apple's limit for apns-collapse-id

const CLOSE_TOO_BIG = 1009;
const CLOSE_REPLACED = 4001;
const CLOSE_DELETED = 4004;
const OPEN = 1; // WebSocket.readyState

const HEX_TOKEN = /^[0-9a-f]{1,512}$/i;
const WS_PATH = /^\/rooms\/([0-9a-f]{32})\/ws$/;

const PEER_ON = JSON.stringify({ t: "peer", on: true });
const PEER_OFF = JSON.stringify({ t: "peer", on: false });

interface Tokens {
  alert: string | null;
  live: string | null;
  sandbox: boolean;
}

// A fixed counting window: `n` events since `start`.
interface Window {
  start: number;
  n: number;
}

// Kept on each socket so it survives hibernation.
interface Attachment {
  room: string;
  frames: Window;
}

export class Room extends DurableObject<Cloudflare.Env> {
  // Registers the room. False if it already exists.
  async create(key: string): Promise<boolean> {
    if ((await this.ctx.storage.get("hash")) !== undefined) return false;
    const now = Date.now();
    await this.ctx.storage.put({ hash: await sha256Hex(key), created: now, seen: now });
    return true;
  }

  // Closes the sockets and erases everything. Returns the HTTP status.
  async destroy(key: string): Promise<204 | 401 | 404> {
    const denied = await this.denied(key);
    if (denied) return denied;
    for (const ws of this.ctx.getWebSockets()) close(ws, CLOSE_DELETED, "room deleted");
    await this.ctx.storage.deleteAll();
    return 204;
  }

  // Opens the socket of one role.
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url);
    const room = WS_PATH.exec(url.pathname)?.[1];
    if (room === undefined) return new Response("not found", { status: 404 });
    const denied = await this.denied(bearerOf(request));
    if (denied === 404) return new Response("unknown room", { status: 404 });
    if (denied === 401) return unauthorized();
    const role = url.searchParams.get("role");
    if (role !== "desktop" && role !== "phone") return new Response("bad role", { status: 400 });
    if (request.headers.get("Upgrade")?.toLowerCase() !== "websocket") {
      return new Response("expected a WebSocket", { status: 426, headers: { Upgrade: "websocket" } });
    }

    // One socket per role: a newer one replaces the older one.
    for (const old of this.ctx.getWebSockets(role)) close(old, CLOSE_REPLACED, "replaced");

    const { 0: client, 1: server } = new WebSocketPair();
    const now = Date.now();
    this.ctx.acceptWebSocket(server, [role]);
    server.serializeAttachment({ room, frames: { start: now, n: 0 } } satisfies Attachment);
    await this.ctx.storage.put("seen", now);

    const peer = this.socket(other(role));
    send(server, peer ? PEER_ON : PEER_OFF);
    if (peer) send(peer, PEER_ON);
    console.log("connect", room, role);
    return new Response(null, { status: 101, webSocket: client });
  }

  async webSocketMessage(ws: WebSocket, message: string | ArrayBuffer): Promise<void> {
    // A socket that was replaced or refused may still have frames on the way.
    if (ws.readyState !== OPEN) return;

    const tooBig =
      typeof message === "string" ? exceeds(message, MAX_FRAME_BYTES) : message.byteLength > MAX_FRAME_BYTES;
    if (tooBig) {
      fail(ws, "too-big");
      close(ws, CLOSE_TOO_BIG, "frame too big");
      return;
    }

    const meta = ws.deserializeAttachment() as Attachment;
    if (!admit(meta.frames, Date.now(), MINUTE_MS, FRAMES_PER_MINUTE)) return fail(ws, "rate");
    ws.serializeAttachment(meta);

    if (typeof message !== "string") return fail(ws, "bad-frame");
    let frame: Record<string, unknown>;
    try {
      const parsed: unknown = JSON.parse(message);
      if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return fail(ws, "bad-frame");
      frame = parsed as Record<string, unknown>;
    } catch {
      return fail(ws, "bad-frame");
    }

    const role = this.role(ws);
    switch (frame.t) {
      case "pair":
      case "paired":
      case "hs1":
      case "hs2":
      case "msg": {
        // Forwarded exactly as received; the relay cannot read what is inside.
        const peer = this.socket(other(role));
        if (peer) send(peer, message);
        else send(ws, PEER_OFF);
        return;
      }
      case "token":
        if (role !== "phone") return fail(ws, "bad-frame");
        return this.saveTokens(ws, frame);
      case "push":
        if (role !== "desktop") return fail(ws, "bad-frame");
        return this.push(ws, meta.room, frame);
      default:
        return fail(ws, "bad-frame");
    }
  }

  async webSocketClose(ws: WebSocket, code: number, reason: string): Promise<void> {
    close(ws, code, reason);
    await this.left(ws);
  }

  async webSocketError(ws: WebSocket): Promise<void> {
    await this.left(ws);
  }

  // A socket went away: tell the other role, unless a newer socket took over.
  private async left(ws: WebSocket): Promise<void> {
    const role = this.role(ws);
    if (this.socket(role, ws)) return;
    const peer = this.socket(other(role));
    if (peer) send(peer, PEER_OFF);
    // Not after the room was deleted: nothing may be written back.
    if ((await this.ctx.storage.get("hash")) !== undefined) await this.ctx.storage.put("seen", Date.now());
    console.log("disconnect", (ws.deserializeAttachment() as Attachment | null)?.room, role);
  }

  private async saveTokens(ws: WebSocket, frame: Record<string, unknown>): Promise<void> {
    const { alert, sandbox } = frame;
    const live = frame.live ?? null;
    if (
      typeof alert !== "string" ||
      !HEX_TOKEN.test(alert) ||
      (live !== null && (typeof live !== "string" || !HEX_TOKEN.test(live))) ||
      typeof sandbox !== "boolean"
    ) {
      return fail(ws, "bad-frame");
    }
    await this.ctx.storage.put("tokens", { alert, live, sandbox } satisfies Tokens);
  }

  private async push(ws: WebSocket, room: string, frame: Record<string, unknown>): Promise<void> {
    const { kind, d, collapse } = frame;
    const urgent = frame.urgent ?? false;
    if (
      (kind !== "alert" && kind !== "live") ||
      typeof d !== "string" ||
      typeof urgent !== "boolean" ||
      (collapse !== undefined &&
        (typeof collapse !== "string" || collapse === "" || exceeds(collapse, MAX_COLLAPSE_BYTES)))
    ) {
      return fail(ws, "bad-frame");
    }
    let state: unknown;
    if (kind === "live") {
      try {
        state = JSON.parse(d);
      } catch {
        return fail(ws, "bad-frame");
      }
    }

    const config = apnsConfig(this.env);
    if (!config) return fail(ws, "no-apns");

    const tokens = await this.ctx.storage.get<Tokens>("tokens");
    const token = tokens?.[kind];
    if (!tokens || !token) return; // nowhere to deliver: dropped silently

    const now = Date.now();
    const pushes = (await this.ctx.storage.get<Window>("pushes")) ?? { start: now, n: 0 };
    if (!admit(pushes, now, HOUR_MS, PUSHES_PER_HOUR)) return fail(ws, "rate");
    await this.ctx.storage.put("pushes", pushes);

    const target = { token, sandbox: tokens.sandbox };
    const push: Push =
      kind === "alert" ? { ...target, kind, d, urgent, collapse, room } : { ...target, kind, state };
    const { outcome, status } = await deliver(config, push, now);
    console.log("push", room, kind, status);

    if (outcome === "gone") {
      // Apple no longer knows this token; the phone registers again when it opens.
      const current = await this.ctx.storage.get<Tokens>("tokens");
      if (current?.[kind] === token) await this.ctx.storage.put("tokens", { ...current, [kind]: null });
    }
  }

  // Why `key` does not open this room, or null if it does.
  private async denied(key: string | null): Promise<401 | 404 | null> {
    const hash = await this.ctx.storage.get<string>("hash");
    if (hash === undefined) return 404;
    if (key === null || !(await sameSecret(await sha256Hex(key), hash))) return 401;
    return null;
  }

  private role(ws: WebSocket): Role {
    return this.ctx.getTags(ws)[0] as Role;
  }

  // The live socket of a role, if any.
  private socket(role: Role, except?: WebSocket): WebSocket | undefined {
    return this.ctx.getWebSockets(role).find((ws) => ws !== except && ws.readyState === OPEN);
  }
}

function other(role: Role): Role {
  return role === "desktop" ? "phone" : "desktop";
}

// Counts one event in the window; false when the cap is already reached.
function admit(window: Window, now: number, span: number, cap: number): boolean {
  if (now < window.start || now - window.start >= span) {
    window.start = now;
    window.n = 0;
  }
  if (window.n >= cap) return false;
  window.n += 1;
  return true;
}

function send(ws: WebSocket, text: string): void {
  try {
    ws.send(text);
  } catch {
    // The socket is closing; its close handler reports the presence change.
  }
}

function fail(ws: WebSocket, code: ErrorCode): void {
  send(ws, JSON.stringify({ t: "error", code }));
}

function close(ws: WebSocket, code: number, reason: string): void {
  try {
    ws.close(code, reason);
  } catch {
    // Already closed, or a code that cannot be sent back (1005, 1006).
  }
}

// Whether `text` takes more than `limit` bytes in UTF-8. Only encodes it when
// its length alone does not settle the question.
function exceeds(text: string, limit: number): boolean {
  if (text.length > limit) return true;
  if (text.length * 3 <= limit) return false;
  return new TextEncoder().encode(text).length > limit;
}

export function unauthorized(): Response {
  return new Response("unauthorized", { status: 401, headers: { "WWW-Authenticate": "Bearer" } });
}

// The token of `Authorization: Bearer <token>`, or null.
export function bearerOf(request: Request): string | null {
  const match = /^Bearer\s+(\S+)$/i.exec(request.headers.get("Authorization") ?? "");
  return match?.[1] ?? null;
}

export async function sha256Hex(text: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}

// Compares two secrets in constant time. Both are hashed first, so the
// comparison is always between equal lengths.
export async function sameSecret(a: string, b: string): Promise<boolean> {
  const encoder = new TextEncoder();
  const [x, y] = await Promise.all([
    crypto.subtle.digest("SHA-256", encoder.encode(a)),
    crypto.subtle.digest("SHA-256", encoder.encode(b)),
  ]);
  return crypto.subtle.timingSafeEqual(x, y);
}
