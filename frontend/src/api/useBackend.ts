import { useEffect, useState } from "react";
import { ApiClient, type Health, resolveBackend } from "./client";

export type BackendState =
  | { phase: "connecting" }
  | { phase: "ready"; client: ApiClient; health: Health }
  | { phase: "failed"; message: string };

const POLL_INTERVAL_MS = 300;
const STARTUP_TIMEOUT_MS = 60_000;

export function useBackend(): BackendState {
  const [state, setState] = useState<BackendState>({ phase: "connecting" });

  useEffect(() => {
    let cancelled = false;

    async function connect() {
      try {
        const client = new ApiClient(await resolveBackend());
        const deadline = Date.now() + STARTUP_TIMEOUT_MS;
        let lastError: unknown;
        while (!cancelled && Date.now() < deadline) {
          try {
            const health = await client.health();
            if (!cancelled) setState({ phase: "ready", client, health });
            return;
          } catch (error) {
            lastError = error;
            await new Promise((resolve) => setTimeout(resolve, POLL_INTERVAL_MS));
          }
        }
        if (!cancelled) {
          setState({ phase: "failed", message: `Backend did not respond: ${String(lastError)}` });
        }
      } catch (error) {
        if (!cancelled) setState({ phase: "failed", message: String(error) });
      }
    }

    void connect();
    return () => {
      cancelled = true;
    };
  }, []);

  return state;
}
