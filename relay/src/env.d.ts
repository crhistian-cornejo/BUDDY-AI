// Bindings and secrets of the Worker (see wrangler.jsonc and README.md).
declare namespace Cloudflare {
  interface Env {
    ROOMS: DurableObjectNamespace<import("./room").Room>;
    OWNER_KEY?: string;
    APNS_KEY_P8?: string;
    APNS_KEY_ID?: string;
    APNS_TEAM_ID?: string;
    APNS_TOPIC?: string;
  }
  interface GlobalProps {
    mainModule: typeof import("./index");
    durableNamespaces: "Room";
  }
}
