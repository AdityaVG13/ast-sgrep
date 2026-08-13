/**
 * Sticky NDJSON Code Mode worker (`asgrep codemode-serve`).
 *
 * One process, one warm Searcher, for the entire Code Mode program — the biggest
 * Amdahl win over per-wave `codemode-batch` spawns.
 */
import { spawn } from "node:child_process";
import { asEnvelope } from "./dispatch.js";
const DEFAULT_MAX_OUTPUT_BYTES = 4 * 1024 * 1024;
export async function startStickyWorker(options) {
    if (options.signal?.aborted) {
        throw new Error("codemode-serve aborted before start");
    }
    const child = spawn(options.binary, ["--root", options.cwd, "codemode-serve"], {
        cwd: options.cwd,
        env: { ...process.env, ...options.env, NO_COLOR: "1" },
        stdio: ["pipe", "pipe", "pipe"],
    });
    const pending = new Map();
    let nextId = 0;
    let closed = false;
    let stderr = "";
    let stdout = Buffer.alloc(0);
    const decoder = new TextDecoder("utf-8", { fatal: true });
    const maxOutputBytes = options.maxOutputBytes ?? DEFAULT_MAX_OUTPUT_BYTES;
    const failAll = (err) => {
        for (const p of pending.values())
            p.reject(err);
        pending.clear();
    };
    const terminate = (err) => {
        if (closed)
            return;
        closed = true;
        options.signal?.removeEventListener("abort", onAbort);
        killChild(child);
        failAll(err);
        child.stdout.destroy();
    };
    child.stdin.on("error", terminate);
    const handleLine = (line) => {
        const trimmed = line.trim();
        if (!trimmed)
            return;
        let msg;
        try {
            msg = JSON.parse(trimmed);
        }
        catch (cause) {
            terminate(new Error(`codemode-serve bad JSON: ${trimmed.slice(0, 200)}`));
            void cause;
            return;
        }
        const type = msg.type;
        if (type === "bye")
            return;
        const id = typeof msg.id === "string" ? msg.id : undefined;
        if (!id)
            return;
        const waiter = pending.get(id);
        if (!waiter)
            return;
        pending.delete(id);
        waiter.resolve(msg);
    };
    child.stdout.on("data", (chunk) => {
        if (closed)
            return;
        const bytes = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
        let offset = 0;
        let newline;
        while ((newline = bytes.indexOf(0x0a, offset)) >= 0) {
            const segment = bytes.subarray(offset, newline);
            const lineBytes = stdout.length + segment.length + 1;
            if (lineBytes > maxOutputBytes) {
                terminate(new Error(`codemode-serve output exceeded ${maxOutputBytes} bytes`));
                return;
            }
            const line = stdout.length === 0
                ? segment
                : Buffer.concat([stdout, segment], stdout.length + segment.length);
            stdout = Buffer.alloc(0);
            try {
                handleLine(decoder.decode(line));
            }
            catch {
                terminate(new Error("codemode-serve output is not valid UTF-8"));
                return;
            }
            if (closed)
                return;
            offset = newline + 1;
        }
        const tail = bytes.subarray(offset);
        if (stdout.length + tail.length > maxOutputBytes) {
            terminate(new Error(`codemode-serve output exceeded ${maxOutputBytes} bytes`));
        }
        else if (tail.length > 0) {
            stdout = stdout.length === 0
                ? Buffer.from(tail)
                : Buffer.concat([stdout, tail], stdout.length + tail.length);
        }
    });
    child.stderr.on("data", (chunk) => {
        stderr += String(chunk);
        if (stderr.length > 8_192)
            stderr = stderr.slice(-8_192);
    });
    child.on("error", (err) => {
        terminate(err);
    });
    child.on("close", (code, signal) => {
        closed = true;
        options.signal?.removeEventListener("abort", onAbort);
        if (pending.size > 0) {
            failAll(new Error(`codemode-serve exited code=${code ?? "null"} signal=${signal ?? "null"} stderr=${stderr.slice(0, 512)}`));
        }
    });
    const onAbort = () => terminate(new Error("codemode-serve aborted"));
    options.signal?.addEventListener("abort", onAbort, { once: true });
    const write = (payload) => {
        if (closed || !child.stdin.writable) {
            return Promise.reject(new Error("codemode-serve is closed"));
        }
        const id = typeof payload.id === "string" ? payload.id : String(nextId++);
        payload.id = id;
        return new Promise((resolve, reject) => {
            pending.set(id, { resolve, reject });
            try {
                child.stdin.write(`${JSON.stringify(payload)}\n`, (err) => {
                    if (err)
                        terminate(err);
                });
            }
            catch (cause) {
                terminate(cause instanceof Error ? cause : new Error(String(cause)));
            }
        });
    };
    const writeWithControls = (payload, label, signal) => {
        if (signal?.aborted)
            return Promise.reject(new Error(`${label} aborted`));
        const response = write(payload);
        return new Promise((resolve, reject) => {
            let timer;
            let settled = false;
            const finish = (action) => {
                if (settled)
                    return;
                settled = true;
                if (timer)
                    clearTimeout(timer);
                signal?.removeEventListener("abort", onRequestAbort);
                action();
            };
            const fail = (err) => finish(() => {
                terminate(err);
                reject(err);
            });
            const onRequestAbort = () => fail(new Error(`${label} aborted`));
            signal?.addEventListener("abort", onRequestAbort, { once: true });
            if (options.timeoutMs && options.timeoutMs > 0) {
                timer = setTimeout(() => fail(new Error(`${label} timed out after ${options.timeoutMs}ms`)), options.timeoutMs);
            }
            response.then((value) => finish(() => resolve(value)), (cause) => finish(() => reject(cause)));
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
                ? msg.results
                : [];
            const out = { results };
            if (typeof msg.mode === "string")
                out.mode = msg.mode;
            if (typeof msg.wall_ms === "number")
                out.wall_ms = msg.wall_ms;
            if (typeof msg.all_ok === "boolean")
                out.all_ok = msg.all_ok;
            return out;
        },
        async end() {
            if (closed)
                return;
            terminate(new Error("codemode-serve ended"));
        },
    };
}
/** One-shot batch via stdin (avoids tempfile). */
export async function runBatchViaStdin(options) {
    if (options.signal?.aborted)
        throw new Error("codemode-batch aborted");
    return new Promise((resolve, reject) => {
        const child = spawn(options.binary, ["codemode-batch", "--requests", "-", "--json"], {
            cwd: options.cwd,
            env: { ...process.env, ...options.env, NO_COLOR: "1" },
            stdio: ["pipe", "pipe", "pipe"],
        });
        let stdout = "";
        let stderr = "";
        let outputBytes = 0;
        let settled = false;
        const maxOutputBytes = options.maxOutputBytes ?? DEFAULT_MAX_OUTPUT_BYTES;
        const cleanup = () => {
            if (timer)
                clearTimeout(timer);
            options.signal?.removeEventListener("abort", onAbort);
        };
        const finish = (action) => {
            if (settled)
                return;
            settled = true;
            cleanup();
            action();
        };
        const fail = (error) => finish(() => {
            killChild(child);
            reject(error);
        });
        child.stdin.on("error", fail);
        const timer = options.timeoutMs && options.timeoutMs > 0
            ? setTimeout(() => {
                fail(new Error(`codemode-batch timed out after ${options.timeoutMs}ms`));
            }, options.timeoutMs)
            : undefined;
        const onAbort = () => {
            fail(new Error("codemode-batch aborted"));
        };
        options.signal?.addEventListener("abort", onAbort, { once: true });
        child.stdout.on("data", (c) => {
            outputBytes += Buffer.byteLength(c);
            if (outputBytes > maxOutputBytes) {
                fail(new Error(`codemode-batch output exceeded ${maxOutputBytes} bytes`));
                return;
            }
            stdout += String(c);
        });
        child.stderr.on("data", (c) => {
            outputBytes += Buffer.byteLength(c);
            if (outputBytes > maxOutputBytes) {
                fail(new Error(`codemode-batch output exceeded ${maxOutputBytes} bytes`));
                return;
            }
            stderr += String(c);
        });
        child.on("error", (err) => {
            fail(err);
        });
        child.on("close", (code) => {
            if (settled)
                return;
            if (code !== 0) {
                fail(new Error(`codemode-batch exited ${code}: ${stderr.slice(0, 512) || stdout.slice(0, 512)}`));
                return;
            }
            try {
                const value = JSON.parse(stdout);
                finish(() => resolve(value));
            }
            catch (cause) {
                fail(cause instanceof Error ? cause : new Error(String(cause)));
            }
        });
        child.stdin.write(options.body, (err) => {
            if (err) {
                fail(err);
                return;
            }
            child.stdin.end();
        });
    });
}
function killChild(child) {
    if (child.exitCode !== null || child.signalCode !== null)
        return;
    try {
        child.kill("SIGTERM");
    }
    catch {
        return;
    }
    setTimeout(() => {
        try {
            if (child.exitCode === null && child.signalCode === null)
                child.kill("SIGKILL");
        }
        catch {
            // ignore
        }
    }, 2_000).unref?.();
}
