import { runInDurableObject } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { describe, expect, it } from "vitest";
import { connect, createRoom, OWNER, pairUp, request, sha256Hex, stub, upgrade } from "./helpers";

describe("GET /", () => {
  it("answers with the relay's name", async () => {
    const res = await request("/");
    expect(res.status).toBe(200);
    expect(await res.text()).toBe("buddy-relay");
  });

  it("answers 404 for unknown paths", async () => {
    expect((await request("/nope", {}, OWNER)).status).toBe(404);
  });
});

describe("POST /rooms", () => {
  it("refuses without the owner key", async () => {
    expect((await request("/rooms", { method: "POST" })).status).toBe(401);
  });

  it("refuses a wrong owner key", async () => {
    expect((await request("/rooms", { method: "POST" }, "not-the-owner-key")).status).toBe(401);
    expect((await request("/rooms", { method: "POST" }, "")).status).toBe(401);
  });

  it("refuses everybody while no owner key is configured", async () => {
    const secrets = env as Partial<Cloudflare.Env>;
    delete secrets.OWNER_KEY;
    try {
      expect((await request("/rooms", { method: "POST" }, OWNER)).status).toBe(401);
      expect((await request("/rooms", { method: "POST" }, "undefined")).status).toBe(401);
      secrets.OWNER_KEY = "";
      expect((await request("/rooms", { method: "POST" }, "")).status).toBe(401);
    } finally {
      secrets.OWNER_KEY = OWNER;
    }
  });

  it("creates a room with a random id and key", async () => {
    const res = await request("/rooms", { method: "POST" }, OWNER);
    expect(res.status).toBe(201);
    expect(res.headers.get("content-type")).toContain("application/json");
    const first = (await res.json()) as { room: string; key: string };
    expect(first.room).toMatch(/^[0-9a-f]{32}$/);
    expect(first.key).toMatch(/^[0-9a-f]{64}$/);

    const second = await createRoom();
    expect(second.room).not.toBe(first.room);
    expect(second.key).not.toBe(first.key);
  });

  it("stores the hash of the room key, never the key", async () => {
    const info = await createRoom();
    const stored = await runInDurableObject(stub(info), async (_room, state) => state.storage.list());
    expect([...stored.keys()].sort()).toEqual(["created", "hash", "seen"]);
    expect(stored.get("hash")).toBe(await sha256Hex(info.key));
    expect(JSON.stringify([...stored.values()])).not.toContain(info.key);
  });
});

describe("GET /rooms/<room>/ws", () => {
  it("refuses a missing or wrong room key", async () => {
    const info = await createRoom();
    expect((await upgrade(info.room, "desktop")).status).toBe(401);
    expect((await upgrade(info.room, "desktop", "0".repeat(64))).status).toBe(401);
    expect((await upgrade(info.room, "desktop", OWNER)).status).toBe(401);
  });

  it("answers 404 for an unknown room", async () => {
    expect((await upgrade("0".repeat(32), "desktop", "0".repeat(64))).status).toBe(404);
    expect((await upgrade("not-a-room", "desktop", "0".repeat(64))).status).toBe(404);
  });

  it("leaves nothing stored for an unknown room", async () => {
    const info = { room: "1".repeat(32), key: "0".repeat(64) };
    await upgrade(info.room, "desktop", info.key);
    const stored = await runInDurableObject(stub(info), async (_room, state) => state.storage.list());
    expect(stored.size).toBe(0);
  });

  it("answers 400 for a bad role", async () => {
    const info = await createRoom();
    expect((await upgrade(info.room, "tablet", info.key)).status).toBe(400);
    expect((await request(`/rooms/${info.room}/ws`, { headers: { Upgrade: "websocket" } }, info.key)).status).toBe(400);
  });

  it("answers 426 when it is not a WebSocket request", async () => {
    const info = await createRoom();
    expect((await request(`/rooms/${info.room}/ws?role=desktop`, {}, info.key)).status).toBe(426);
  });

  it("opens a socket with the room key", async () => {
    const info = await createRoom();
    const desktop = await connect(info, "desktop");
    expect(await desktop.next()).toEqual({ t: "peer", on: false });
  });
});

describe("DELETE /rooms/<room>", () => {
  it("refuses a missing or wrong room key", async () => {
    const info = await createRoom();
    expect((await request(`/rooms/${info.room}`, { method: "DELETE" })).status).toBe(401);
    expect((await request(`/rooms/${info.room}`, { method: "DELETE" }, "0".repeat(64))).status).toBe(401);
    // Still there.
    await connect(info, "desktop");
  });

  it("answers 404 for an unknown room", async () => {
    expect((await request(`/rooms/${"2".repeat(32)}`, { method: "DELETE" }, "0".repeat(64))).status).toBe(404);
  });

  it("closes the sockets and wipes the storage", async () => {
    const { info, desktop, phone } = await pairUp();
    phone.send({ t: "token", alert: "ab12", live: null, sandbox: false });
    phone.send({ t: "msg", d: "sync" });
    expect(await desktop.next()).toEqual({ t: "msg", d: "sync" });

    const res = await request(`/rooms/${info.room}`, { method: "DELETE" }, info.key);
    expect(res.status).toBe(204);
    expect((await desktop.closed).code).toBe(4004);
    expect((await phone.closed).code).toBe(4004);

    const stored = await runInDurableObject(stub(info), async (_room, state) => state.storage.list());
    expect(stored.size).toBe(0);
    expect((await upgrade(info.room, "desktop", info.key)).status).toBe(404);
  });
});
