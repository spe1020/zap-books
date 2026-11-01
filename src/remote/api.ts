/**
 * The ledger service's HTTP API (`docs/service.md`). Reads use a session
 * token from `POST /v1/session`, held in memory only; a 401 signs in again
 * once. Proposals need no session: they are signed events.
 */
import type { NostrEvent } from "./protocol";
import type { Signer } from "./signer";

/** The service couldn't be reached: the command waits in the queue. */
export class Offline extends Error {
  constructor() {
    super("The service can't be reached. Check your connection.");
  }
}

/** The service answered with an error status. */
export class ServiceError extends Error {
  constructor(
    readonly status: number,
    message: string,
    readonly body?: Record<string, unknown>,
  ) {
    super(message);
  }
}

/** The service's answer to a proposal: its signed acknowledgment. */
export interface Acknowledgment {
  status: "accepted" | "rejected";
  event: NostrEvent;
}

function parse(text: string): Record<string, unknown> | undefined {
  try {
    return JSON.parse(text);
  } catch {
    return undefined;
  }
}

/** The service's error answer, with its own message when it gave one. */
function failure(status: number, body?: Record<string, unknown>) {
  return new ServiceError(
    status,
    typeof body?.error === "string"
      ? body.error
      : `The service answered ${status}.`,
    body,
  );
}

const base64 = (text: string) =>
  btoa(String.fromCharCode(...new TextEncoder().encode(text)));

export class ServiceApi {
  private token?: { value: string; expiresAt: number };
  constructor(
    readonly url: string,
    private readonly signer: Signer,
  ) {}

  private async fetch(path: string, init: RequestInit): Promise<Response> {
    try {
      return await fetch(`${this.url}${path}`, {
        ...init,
        credentials: "omit",
        cache: "no-store",
      });
    } catch {
      throw new Offline();
    }
  }

  private async json<T>(response: Response): Promise<T> {
    const text = await response.text();
    const body = parse(text);
    if (!response.ok) throw failure(response.status, body);
    if (body === undefined)
      throw new ServiceError(
        response.status,
        "The service's answer wasn't JSON.",
      );
    return body as T;
  }

  /** The key every new business must name. */
  async serviceKey(): Promise<string> {
    const r = await this.json<{ pubkey: string }>(
      await this.fetch("/v1/service", { method: "GET" }),
    );
    return r.pubkey;
  }

  /** The second of the last NIP-98 event, per URL and method. */
  private signedAt = new Map<string, number>();

  /**
   * NIP-98: a signed kind-27235 event naming this exact URL and method. Two
   * signed within one second would be the same event, which the service
   * refuses as a replay, so the second waits for the next second.
   */
  private async nip98(path: string, method: string): Promise<string> {
    const key = `${method} ${path}`;
    let at = Math.floor(Date.now() / 1000);
    const last = this.signedAt.get(key);
    if (last !== undefined && at <= last) {
      await new Promise((r) => setTimeout(r, (last + 1) * 1000 - Date.now()));
      at = last + 1;
    }
    this.signedAt.set(key, at);
    const event = await this.signer.sign({
      kind: 27235,
      created_at: at,
      tags: [
        ["u", `${this.url}${path}`],
        ["method", method],
      ],
      content: "",
    });
    return `Nostr ${base64(JSON.stringify(event))}`;
  }

  /**
   * Signs in for reads: one signature per hour, not one per request. An
   * auth event the service has seen (one signed in the same second, from a
   * page since reloaded) is signed again a second later, once.
   */
  async signIn(): Promise<void> {
    for (let attempt = 0; ; attempt++) {
      try {
        const r = await this.json<{ token: string; expiresAt: number }>(
          await this.fetch("/v1/session", {
            method: "POST",
            headers: { Authorization: await this.nip98("/v1/session", "POST") },
          }),
        );
        this.token = { value: r.token, expiresAt: r.expiresAt };
        return;
      } catch (e) {
        if (!(e instanceof ServiceError) || e.status !== 401 || attempt > 0)
          throw e;
      }
    }
  }

  /** The session token, signing in first if it is missing or ending. */
  private async session(): Promise<string> {
    const fresh = this.token && this.token.expiresAt - 60 > Date.now() / 1000;
    if (!fresh) await this.signIn();
    return this.token!.value;
  }

  /**
   * `send` with the session token; on a 401 (the session ended early), signs
   * in again and sends once more.
   */
  private async withSession<R extends { status: number }>(
    send: (token: string) => Promise<R>,
  ): Promise<R> {
    for (let attempt = 0; ; attempt++) {
      const answer = await send(await this.session());
      if (answer.status === 401 && attempt === 0) {
        this.token = undefined;
        continue;
      }
      return answer;
    }
  }

  /** A read with the session token, signing in first or again as needed. */
  async read<T>(path: string): Promise<T> {
    const response = await this.withSession((token) =>
      this.fetch(path, {
        method: "GET",
        headers: { Authorization: `Bearer ${token}` },
      }),
    );
    return this.json<T>(response);
  }

  /**
   * Uploads an encrypted receipt (`PUT /v1/blobs/{hash}`). Uploading one the
   * service already has succeeds. `progress` hears the bytes sent so far.
   * XMLHttpRequest, since fetch reports no upload progress.
   */
  async upload(
    business: string,
    hash: string,
    ciphertext: Uint8Array<ArrayBuffer>,
    progress?: (sent: number, total: number) => void,
  ): Promise<void> {
    const url = `${this.url}/v1/blobs/${hash}?business=${business}`;
    const answer = await this.withSession(
      (token) =>
        new Promise<{ status: number; text: string }>((resolve, reject) => {
          const xhr = new XMLHttpRequest();
          xhr.open("PUT", url);
          xhr.setRequestHeader("Authorization", `Bearer ${token}`);
          xhr.setRequestHeader("Content-Type", "application/octet-stream");
          xhr.upload.onprogress = (e) => progress?.(e.loaded, e.total);
          xhr.onload = () =>
            resolve({ status: xhr.status, text: xhr.responseText });
          xhr.onerror =
            xhr.onabort =
            xhr.ontimeout =
              () => reject(new Offline());
          xhr.send(ciphertext);
        }),
    );
    if (answer.status < 200 || answer.status >= 300)
      throw failure(answer.status, parse(answer.text));
  }

  /** Downloads an encrypted receipt (`GET /v1/blobs/{hash}`). */
  async download(business: string, hash: string): Promise<ArrayBuffer> {
    const response = await this.withSession((token) =>
      this.fetch(`/v1/blobs/${hash}?business=${business}`, {
        method: "GET",
        headers: { Authorization: `Bearer ${token}` },
      }),
    );
    if (!response.ok)
      throw failure(response.status, parse(await response.text()));
    try {
      return await response.arrayBuffer();
    } catch {
      throw new Offline();
    }
  }

  /** Sends a signed proposal. Resending it returns the same answer. */
  async propose(event: NostrEvent): Promise<Acknowledgment> {
    return this.json<Acknowledgment>(
      await this.fetch("/v1/proposals", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(event),
      }),
    );
  }

  /** Forgets the session. */
  signOut() {
    this.token = undefined;
  }
}
