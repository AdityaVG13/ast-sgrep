/**
 * Sticky NDJSON Code Mode worker (`asgrep codemode-serve`).
 *
 * One process, one warm Searcher, for the entire Code Mode program — the biggest
 * Amdahl win over per-wave `codemode-batch` spawns.
 */

import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { createInterface, type Interface } from "node:readline";
import type { MachineEnvelope } from "../runtime.js";
import { asEnvelope, type BatchResult, type StickyWorker } from "./dispatch.js";

export type StickyWorkerOptions = {
  binary: string;
  cwd: string;
  env?: NodeJS.ProcessEnv;
  signal?: AbortSignal;
  /** Kill worker when one request exceeds this duration (ms). */
  timeoutMs?: number;
};

type Pending = {
  resolve: (value: Record<string, unknown>) => void;
  reject: (reason: unknown) => void;
};

export async function startStickyWorker(options: StickyWorkerOptions): Promise<StickyWorker> {
  if (options.signal?.aborted) {
    throw new Error("codemode-serve aborted before start");
  }

  const child: ChildProcessWithoutNullStreams = spawn(
    options.binary,
    ["--root", options.cwd, "codemode-serve"],
    {
      cwd: options.cwd,
      env: { ...process.env, ...options.env, NO_COLOR: "1" },
      stdio: ["pipe", "pipe", "pipe"],
    },
  );

  const pending = new Map<string, Pending>();
  let nextId = 0;
  let closed = false;
  let stderr = "";

  const rl: Interface = createInterface({ input: child.stdout, crlfDelay: Infinity });

  const failAll = (err: Error) => {
    for (const p of pending.values()) p.reject(err);
    pending.clear();
  };

  const terminate = (err: Error) => {
    if (closed) return;
    closed = true;
    killChild(child);
    failAll(err);
    rl.close();
  };

  rl.on("line", (line) => {
    const trimmed = line.trim();
    if (!trimmed) return;
    let msg: Record<string, unknown>;
    try {
      msg = JSON.parse(trimmed) as Record<string, unknown>;
    } catch (cause) {
      failAll(new Error(`codemode-serve bad JSON: ${trimmed.slice(0, 200)}`));
      void cause;
      return;
    }
    const type = msg.type;
    if (type === "bye") return;
    const id = typeof msg.id === "string" ? msg.id : undefined;
    if (!id) return;
    const waiter = pending.get(id);
    if (!waiter) return;
    pending.delete(id);
    waiter.resolve(msg);
  });

  child.stderr.on("data", (chunk: Buffer | string) => {
    stderr += String(chunk);
    if (stderr.length > 8_192) stderr = stderr.slice(-8_192);
  });

  child.on("error", (err) => {
    closed = true;
    failAll(err);
  });

  child.on("close", (code, signal) => {
    closed = true;
    if (pending.size > 0) {
      failAll(
        new Error(
          `codemode-serve exited code=${code ?? "null"} signal=${signal ?? "null"} stderr=${stderr.slice(0, 512)}`,
        ),
      );
    }
  });

  const onAbort = () => terminate(new Error("codemode-serve aborted"));
  options.signal?.addEventListener("abort", onAbort, { once: true });

  const write = (payload: Record<string, unknown>): Promise<Record<string, unknown>> => {
    if (closed || !child.stdin.writable) {
      return Promise.reject(new Error("codemode-serve is closed"));
    }
    const id = typeof payload.id === "string" ? payload.id : String(nextId++);
    payload.id = id;
    return new Promise((resolve, reject) => {
      pending.set(id, { resolve, reject });
      child.stdin.write(`${JSON.stringify(payload)}\n`, (err) => {
        if (err) {
          pending.delete(id);
          reject(err);
        }
      });
    });
  };

  const writeWithControls = (
    payload: Record<string, unknown>,
    label: string,
    signal?: AbortSignal,
  ): Promise<Record<string, unknown>> => {
    if (signal?.aborted) return Promise.reject(new Error(`${label} aborted`));
    const response = write(payload);
    return new Promise((resolve, reject) => {
      let timer: ReturnType<typeof setTimeout> | undefined;
      let settled = false;
      const finish = (action: () => void) => {
        if (settled) return;
        settled = true;
        if (timer) clearTimeout(timer);
        signal?.removeEventListener("abort", onRequestAbort);
        action();
      };
      const fail = (err: Error) => finish(() => {
        terminate(err);
        reject(err);
      });
      const onRequestAbort = () => fail(new Error(`${label} aborted`));
      signal?.addEventListener("abort", onRequestAbort, { once: true });
      if (options.timeoutMs && options.timeoutMs > 0) {
        timer = setTimeout(
          () => fail(new Error(`${label} timed out after ${options.timeoutMs}ms`)),
          options.timeoutMs,
        );
      }
      response.then(
        (value) => finish(() => resolve(value)),
        (cause) => finish(() => reject(cause)),
      );
    });
  };

  // Probe: empty End would close — instead send a tiny catalog call to verify protocol,
  // or just return and let first real call fail. Prefer lazy: no probe.

  return {
    async call(tool, args, callOptions) {
      const msg = await writeWithControls({ type: "call", tool, args }, "codemode call", callOptions?.signal);
      if (msg.type === "error") {
        throw new Error(typeof msg.error === "string" ? msg.error : "codemode-serve error");
      }
      if (msg.ok === false) {
        throw new Error(typeof msg.error === "string" ? msg.error : `codemode ${tool} failed`);
      }
      return asEnvelope(msg.value, tool);
    },

    async batch(calls, callOptions) {
      const msg = await writeWithControls({ type: "batch", calls }, "codemode batch", callOptions?.signal);
      if (msg.type === "error") {
        throw new Error(typeof msg.error === "string" ? msg.error : "codemode-serve batch error");
      }
      const results = Array.isArray(msg.results)
        ? (msg.results as BatchResult["results"])
        : [];
      const out: BatchResult = { results };
      if (typeof msg.mode === "string") out.mode = msg.mode;
      if (typeof msg.wall_ms === "number") out.wall_ms = msg.wall_ms;
      if (typeof msg.all_ok === "boolean") out.all_ok = msg.all_ok;
      return out;
    },

    async end() {
      options.signal?.removeEventListener("abort", onAbort);
      if (closed) return;
      try {
        if (child.stdin.writable) {
          child.stdin.write(`${JSON.stringify({ type: "end" })}\n`);
          child.stdin.end();
        }
      } catch {
        // ignore
      }
      killChild(child);
      closed = true;
      rl.close();
    },
  };
}

/** One-shot batch via stdin (avoids tempfile). */
export async function runBatchViaStdin(options: {
  binary: string;
  cwd: string;
  body: string;
  env?: NodeJS.ProcessEnv;
  signal?: AbortSignal;
  timeoutMs?: number;
}): Promise<MachineEnvelope> {
  if (options.signal?.aborted) throw new Error("codemode-batch aborted");
  return new Promise((resolve, reject) => {
    const child = spawn(
      options.binary,
      ["codemode-batch", "--requests", "-", "--json"],
      {
        cwd: options.cwd,
        env: { ...process.env, ...options.env, NO_COLOR: "1" },
        stdio: ["pipe", "pipe", "pipe"],
      },
    );
    let stdout = "";
    let stderr = "";
    const timer =
      options.timeoutMs && options.timeoutMs > 0
        ? setTimeout(() => {
            killChild(child);
            reject(new Error(`codemode-batch timed out after ${options.timeoutMs}ms`));
          }, options.timeoutMs)
        : undefined;

    const onAbort = () => {
      killChild(child);
      reject(new Error("codemode-batch aborted"));
    };
    options.signal?.addEventListener("abort", onAbort, { once: true });

    child.stdout.on("data", (c: Buffer | string) => {
      stdout += String(c);
    });
    child.stderr.on("data", (c: Buffer | string) => {
      stderr += String(c);
    });
    child.on("error", (err) => {
      if (timer) clearTimeout(timer);
      options.signal?.removeEventListener("abort", onAbort);
      reject(err);
    });
    child.on("close", (code) => {
      if (timer) clearTimeout(timer);
      options.signal?.removeEventListener("abort", onAbort);
      if (code !== 0) {
        reject(new Error(`codemode-batch exited ${code}: ${stderr.slice(0, 512) || stdout.slice(0, 512)}`));
        return;
      }
      try {
        resolve(JSON.parse(stdout) as MachineEnvelope);
      } catch (cause) {
        reject(cause);
      }
    });
    child.stdin.write(options.body, (err) => {
      if (err) {
        killChild(child);
        reject(err);
        return;
      }
      child.stdin.end();
    });
  });
}

function killChild(child: ChildProcessWithoutNullStreams): void {
  if (child.exitCode !== null || child.signalCode !== null) return;
  try {
    child.kill("SIGTERM");
  } catch {
    return;
  }
  setTimeout(() => {
    try {
      if (child.exitCode === null && child.signalCode === null) child.kill("SIGKILL");
    } catch {
      // ignore
    }
  }, 2_000).unref?.();
}
