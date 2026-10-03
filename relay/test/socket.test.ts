import { evictDurableObject, runInDurableObject } from "cloudflare:test";
import { describe, expect, it } from "vitest";
import { connect, createRoom, pairUp, stub } from "./helpers";

const BAD_FRAME = { t: "error", code: "bad-frame" };

describe("presence", () => {
  it("tells each side whether the other is there", async () => {
    const info = await createRoom();
    const desktop = await connect(info, "desktop");
    expect(await desktop.next()).toEqual({ t: "peer", on: false });

    const phone = await connect(info, "phone");
    expect(await phone.next()).toEqual({ t: "peer", on: true });
    expect(await desktop.next()).toEqual({ t: "peer", on: true });

    phone.close();
    expect(await desktop.next()).toEqual({ t: "peer", on: false });

    const again = await connect(info, "phone");
    expect(await again.next()).toEqual({ t: "peer", on: true });
    expect(await desktop.next()).toEqual({ t: "peer", on: true });

    desktop.close();
    expect(await again.next()).toEqual({ t: "peer", on: false });
  });

  it("records when the room was last seen", async () => {
    const info = await createRoom();
    const before = Date.now();
    await connect(info, "desktop");
    const seen = await runInDurableObject(stub(info), (_room, state) => state.storage.get<number>("seen"));
    expect(seen).toBeGreaterThanOrEqual(before);
  });
});

describe("forwarding", () => {
  it("passes frames verbatim from the desktop to the phone", async () => {
    const { desktop, phone } = await pairUp();
    for (const t of ["pair", "paired", "hs1", "hs2", "msg"]) {
      // Odd spacing and extra fields must survive: the relay does not rewrite.
      const frame = `{ "t":"${t}",  "d":"b3BhcXVl", "extra":[1,2,3] }`;
      desktop.send(frame);
      expect(await phone.nextText()).toBe(frame);
    }
  });

  it("passes frames verbatim from the phone to the desktop", async () => {
    const { desktop, phone } = await pairUp();
    for (const t of ["pair", "paired", "hs1", "hs2", "msg"]) {
      const frame = `{"d":"b3BhcXVl","t":"${t}"}`;
      phone.send(frame);
      expect(await desktop.nextText()).toBe(frame);
    }
  });

  it("tells the sender when the other side is not connected", async () => {
    const info = await createRoom();
    const desktop = await connect(info, "desktop");
    expect(await desktop.next()).toEqual({ t: "peer", on: false });
    desktop.send({ t: "msg", d: "aGk=" });
    expect(await desktop.next()).toEqual({ t: "peer", on: false });
  });

  it("keeps rooms apart", async () => {
    const one = await pairUp();
    const two = await pairUp();
    one.desktop.send({ t: "msg", d: "one" });
    two.desktop.send({ t: "msg", d: "two" });
    expect(await one.phone.next()).toEqual({ t: "msg", d: "one" });
    expect(await two.phone.next()).toEqual({ t: "msg", d: "two" });
  });
});

describe("socket replacement", () => {
  it("closes the first socket of a role with 4001 when a second one arrives", async () => {
    const { info, desktop, phone } = await pairUp();

    const newer = await connect(info, "desktop");
    expect((await desktop.closed).code).toBe(4001);
    expect(await newer.next()).toEqual({ t: "peer", on: true });
    // The phone hears about the new connection and never sees the role as gone.
    expect(await phone.next()).toEqual({ t: "peer", on: true });

    phone.send({ t: "msg", d: "to-newer" });
    expect(await newer.next()).toEqual({ t: "msg", d: "to-newer" });
    newer.send({ t: "msg", d: "from-newer" });
    expect(await phone.next()).toEqual({ t: "msg", d: "from-newer" });
  });
});

describe("limits", () => {
  it("forwards a frame of exactly 256 KB", async () => {
    const { desktop, phone } = await pairUp();
    const frame = JSON.stringify({ t: "msg", d: "" });
    const full = JSON.stringify({ t: "msg", d: "a".repeat(256 * 1024 - frame.length) });
    expect(full.length).toBe(256 * 1024);
    desktop.send(full);
    expect(await phone.nextText()).toBe(full);
  });

  it("refuses a larger frame with too-big and closes with 1009", async () => {
    const { desktop, phone } = await pairUp();
    desktop.send(JSON.stringify({ t: "msg", d: "a".repeat(256 * 1024) }));
    expect(await desktop.next()).toEqual({ t: "error", code: "too-big" });
    expect((await desktop.closed).code).toBe(1009);
    // Nothing was forwarded: the phone only learns that the desktop left.
    expect(await phone.next()).toEqual({ t: "peer", on: false });
  });

  it("counts multi-byte text in bytes", async () => {
    const { desktop } = await pairUp();
    // 100 000 characters, 300 000 bytes in UTF-8.
    desktop.send(JSON.stringify({ t: "msg", d: "€".repeat(100_000) }));
    expect(await desktop.next()).toEqual({ t: "error", code: "too-big" });
    expect((await desktop.closed).code).toBe(1009);
  });

  it("drops frames beyond 600 per minute with rate", async () => {
    const info = await createRoom();
    const desktop = await connect(info, "desktop");
    expect(await desktop.next()).toEqual({ t: "peer", on: false });

    for (let i = 0; i < 601; i++) desktop.send({ t: "msg", d: "x" });
    for (let i = 0; i < 600; i++) expect(await desktop.next()).toEqual({ t: "peer", on: false });
    expect(await desktop.next()).toEqual({ t: "error", code: "rate" });
  });

  it("counts each socket on its own", async () => {
    const { desktop, phone } = await pairUp();
    for (let i = 0; i < 600; i++) desktop.send({ t: "msg", d: "x" });
    for (let i = 0; i < 600; i++) await phone.next();
    phone.send({ t: "msg", d: "still-fine" });
    expect(await desktop.next()).toEqual({ t: "msg", d: "still-fine" });
  });
});

describe("bad frames", () => {
  it("refuses text that is not a JSON object with a known t", async () => {
    const { desktop, phone } = await pairUp();
    const bad = ["not json", "[1,2]", "null", '"msg"', "{}", '{"t":7}', '{"t":"nope"}', '{"t":"peer","on":true}', '{"t":"error","code":"rate"}'];
    for (const frame of bad) {
      desktop.send(frame);
      expect(await desktop.next(), frame).toEqual(BAD_FRAME);
    }
    // None of them reached the phone.
    desktop.send({ t: "msg", d: "ok" });
    expect(await phone.next()).toEqual({ t: "msg", d: "ok" });
  });

  it("refuses binary frames", async () => {
    const { desktop } = await pairUp();
    desktop.send(new Uint8Array([1, 2, 3]).buffer);
    expect(await desktop.next()).toEqual(BAD_FRAME);
  });
});

describe("token", () => {
  const tokens = (info: { room: string; key: string }) =>
    runInDurableObject(stub(info), (_room, state) => state.storage.get("tokens"));

  it("stores the phone's tokens and does not forward them", async () => {
    const { info, desktop, phone } = await pairUp();
    phone.send({ t: "token", alert: "ab12cd34", live: "ef56", sandbox: true });
    phone.send({ t: "msg", d: "after" });
    expect(await desktop.next()).toEqual({ t: "msg", d: "after" });
    expect(await tokens(info)).toEqual({ alert: "ab12cd34", live: "ef56", sandbox: true });

    phone.send({ t: "token", alert: "ab12cd34", live: null, sandbox: false });
    phone.send({ t: "msg", d: "again" });
    expect(await desktop.next()).toEqual({ t: "msg", d: "again" });
    expect(await tokens(info)).toEqual({ alert: "ab12cd34", live: null, sandbox: false });
  });

  it("refuses tokens from the desktop", async () => {
    const { info, desktop } = await pairUp();
    desktop.send({ t: "token", alert: "ab12cd34", live: null, sandbox: false });
    expect(await desktop.next()).toEqual(BAD_FRAME);
    expect(await tokens(info)).toBeUndefined();
  });

  it("refuses tokens that are not hex", async () => {
    const { info, phone } = await pairUp();
    const bad = [
      { t: "token", alert: "../../3/device/x", live: null, sandbox: false },
      { t: "token", alert: "", live: null, sandbox: false },
      { t: "token", alert: "ab12", live: "zz", sandbox: false },
      { t: "token", alert: "ab12", live: null, sandbox: "yes" },
      { t: "token", alert: 12, live: null, sandbox: false },
    ];
    for (const frame of bad) {
      phone.send(frame);
      expect(await phone.next(), JSON.stringify(frame)).toEqual(BAD_FRAME);
    }
    expect(await tokens(info)).toBeUndefined();
  });
});

// The room sleeps between frames (WebSocket hibernation); nothing it needs may
// live only in memory.
describe("after hibernation", () => {
  it("still forwards, reports presence and replaces sockets", async () => {
    const { info, desktop, phone } = await pairUp();
    await evictDurableObject(stub(info));

    desktop.send({ t: "msg", d: "woke" });
    expect(await phone.next()).toEqual({ t: "msg", d: "woke" });

    await evictDurableObject(stub(info));
    const newer = await connect(info, "phone");
    expect((await phone.closed).code).toBe(4001);
    expect(await newer.next()).toEqual({ t: "peer", on: true });
    expect(await desktop.next()).toEqual({ t: "peer", on: true });

    await evictDurableObject(stub(info));
    newer.close();
    expect(await desktop.next()).toEqual({ t: "peer", on: false });
  });

  it("keeps the frame count of each socket", async () => {
    const { info, desktop, phone } = await pairUp();
    for (let i = 0; i < 600; i++) desktop.send({ t: "msg", d: "x" });
    for (let i = 0; i < 600; i++) await phone.next();
    await evictDurableObject(stub(info));

    desktop.send({ t: "msg", d: "x" });
    expect(await desktop.next()).toEqual({ t: "error", code: "rate" });
  });

  it("still refuses frames by role", async () => {
    const { info, desktop, phone } = await pairUp();
    await evictDurableObject(stub(info));
    desktop.send({ t: "token", alert: "ab12", live: null, sandbox: false });
    expect(await desktop.next()).toEqual(BAD_FRAME);
    phone.send({ t: "push", kind: "alert", d: "eA==", urgent: false });
    expect(await phone.next()).toEqual(BAD_FRAME);
  });
});
