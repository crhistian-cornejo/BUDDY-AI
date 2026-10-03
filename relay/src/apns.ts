// Delivery of push notifications to Apple (APNs) with a provider token.

export interface ApnsConfig {
  key: string; // PKCS#8 private key, the contents of Apple's .p8 file
  keyId: string;
  teamId: string;
  topic: string; // the iPhone app's bundle id
}

export type Push = { token: string; sandbox: boolean } & (
  | { kind: "alert"; d: string; urgent: boolean; collapse?: string; room: string }
  | { kind: "live"; state: unknown }
);

// "gone": Apple no longer knows the device token, so the caller forgets it.
export type Outcome = "sent" | "gone" | "failed";

const TOKEN_LIFE_MS = 40 * 60 * 1000;
const TIMEOUT_MS = 10_000;

// The APNs settings, or null unless all four secrets are set.
export function apnsConfig(env: Cloudflare.Env): ApnsConfig | null {
  const { APNS_KEY_P8: key, APNS_KEY_ID: keyId, APNS_TEAM_ID: teamId, APNS_TOPIC: topic } = env;
  if (!key || !keyId || !teamId || !topic) return null;
  return { key, keyId, teamId, topic };
}

let cached: { jwt: string; at: number; config: ApnsConfig } | null = null;

// The signed provider token (ES256 JWT). Apple refuses tokens that are renewed
// too often or older than an hour, so one is kept for 40 minutes.
export async function providerToken(config: ApnsConfig, now: number): Promise<string> {
  if (
    cached &&
    now >= cached.at &&
    now - cached.at < TOKEN_LIFE_MS &&
    cached.config.key === config.key &&
    cached.config.keyId === config.keyId &&
    cached.config.teamId === config.teamId
  ) {
    return cached.jwt;
  }
  const header = base64Url(utf8(JSON.stringify({ alg: "ES256", kid: config.keyId })));
  const claims = base64Url(utf8(JSON.stringify({ iss: config.teamId, iat: Math.floor(now / 1000) })));
  const key = await crypto.subtle.importKey(
    "pkcs8",
    pemToDer(config.key),
    { name: "ECDSA", namedCurve: "P-256" },
    false,
    ["sign"],
  );
  // WebCrypto returns the raw r||s signature, which is the form a JWT wants.
  const signature = await crypto.subtle.sign({ name: "ECDSA", hash: "SHA-256" }, key, utf8(`${header}.${claims}`));
  const jwt = `${header}.${claims}.${base64Url(new Uint8Array(signature))}`;
  cached = { jwt, at: now, config };
  return jwt;
}

// Sends one push. Never throws: a failure is logged by the caller as a status.
export async function deliver(config: ApnsConfig, push: Push, now: number): Promise<{ outcome: Outcome; status: number }> {
  let response: Response;
  try {
    const host = push.sandbox ? "api.sandbox.push.apple.com" : "api.push.apple.com";
    const headers: Record<string, string> = {
      authorization: `bearer ${await providerToken(config, now)}`,
      "content-type": "application/json",
    };
    let body: unknown;
    if (push.kind === "alert") {
      headers["apns-topic"] = config.topic;
      headers["apns-push-type"] = "alert";
      headers["apns-priority"] = "10";
      if (push.collapse !== undefined) headers["apns-collapse-id"] = push.collapse;
      body = {
        aps: {
          alert: { title: "Buddy", body: "Novedad en tu equipo" },
          "mutable-content": 1,
          sound: "default",
          "interruption-level": push.urgent ? "time-sensitive" : "active",
          "thread-id": push.room,
        },
        d: push.d,
      };
    } else {
      headers["apns-topic"] = `${config.topic}.push-type.liveactivity`;
      headers["apns-push-type"] = "liveactivity";
      body = { aps: { timestamp: Math.floor(now / 1000), event: "update", "content-state": push.state } };
    }
    response = await fetch(`https://${host}/3/device/${push.token}`, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
      signal: AbortSignal.timeout(TIMEOUT_MS),
    });
  } catch {
    return { outcome: "failed", status: 0 };
  }

  const status = response.status;
  if (status === 200) return { outcome: "sent", status };
  if (status === 410) return { outcome: "gone", status };
  if (status === 400) {
    const reason = await response
      .json<{ reason?: unknown }>()
      .then((error) => error?.reason)
      .catch(() => undefined);
    if (reason === "BadDeviceToken" || reason === "Unregistered") return { outcome: "gone", status };
  }
  return { outcome: "failed", status };
}

function utf8(text: string): Uint8Array {
  return new TextEncoder().encode(text);
}

function base64Url(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

// Accepts the .p8 file as is, or pasted with its line breaks written as "\n".
function pemToDer(pem: string): Uint8Array {
  const base64 = pem
    .replace(/-----[A-Z ]+-----/g, "")
    .replace(/\\n/g, "")
    .replace(/\s+/g, "");
  return Uint8Array.from(atob(base64), (c) => c.charCodeAt(0));
}
