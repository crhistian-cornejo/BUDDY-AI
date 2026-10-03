// Buddy's relay: the meeting point between a desktop and its iPhone.
//
// Everything that passes through is end-to-end encrypted by the clients. The
// relay forwards opaque frames, tracks presence and hands pushes to Apple.
// Never log frame contents, keys or tokens here; room id and event kind only.

import { bearerOf, sameSecret, unauthorized } from "./room";

export { Room } from "./room";

const ROOM_PATH = /^\/rooms\/([0-9a-f]{32})(\/ws)?$/;

export default {
  async fetch(request: Request, env: Cloudflare.Env): Promise<Response> {
    const { pathname } = new URL(request.url);
    const method = request.method;

    if (pathname === "/") {
      return method === "GET" ? new Response("buddy-relay") : notAllowed("GET");
    }

    const bearer = bearerOf(request);

    if (pathname === "/rooms") {
      if (method !== "POST") return notAllowed("POST");
      // Without an owner key configured nobody can create rooms.
      if (!env.OWNER_KEY || bearer === null || !(await sameSecret(bearer, env.OWNER_KEY))) return unauthorized();
      const room = randomHex(16);
      const key = randomHex(32);
      if (!(await env.ROOMS.get(env.ROOMS.idFromName(room)).create(key))) {
        return new Response("try again", { status: 500 });
      }
      console.log("room created", room);
      return Response.json({ room, key }, { status: 201 });
    }

    const match = ROOM_PATH.exec(pathname);
    if (!match) return new Response("not found", { status: 404 });
    if (bearer === null) return unauthorized();
    const room = match[1]!;
    const stub = env.ROOMS.get(env.ROOMS.idFromName(room));

    if (match[2]) {
      // The room checks the key and the role, then takes the socket.
      return method === "GET" ? stub.fetch(request) : notAllowed("GET");
    }

    if (method !== "DELETE") return notAllowed("DELETE");
    const status = await stub.destroy(bearer);
    if (status === 401) return unauthorized();
    if (status === 204) console.log("room deleted", room);
    return new Response(null, { status });
  },
} satisfies ExportedHandler<Cloudflare.Env>;

function notAllowed(allow: string): Response {
  return new Response("method not allowed", { status: 405, headers: { Allow: allow } });
}

function randomHex(bytes: number): string {
  const buffer = crypto.getRandomValues(new Uint8Array(bytes));
  return [...buffer].map((byte) => byte.toString(16).padStart(2, "0")).join("");
}
