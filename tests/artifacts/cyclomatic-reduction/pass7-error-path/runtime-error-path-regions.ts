// Pass 7 extracted error-path regions (runtime.ts)

function throwNonzeroProcessFailure(result: ExecResult, code: number): never {
  try {
    const value = record(JSON.parse(result.stdout) as unknown);
    // Wire-valid ok:false asgrep envelope → structured operational failure (not PROCESS_FAILED).
    if (value && value.tool === "asgrep" && value.schema_version === MACHINE_SCHEMA_VERSION && value.ok === false) {
      const failure = record(value.error);
      const message = typeof failure?.message === "string" ? failure.message : "ast-sgrep reported an operational failure";
      throw new RuntimeError("OPERATIONAL_ERROR", message, { command: value.command, error: failure, exitCode: code });
    }
  } catch (cause) {
    if (cause instanceof RuntimeError) throw cause;
  }
  throw new RuntimeError("PROCESS_FAILED", `ast-sgrep exited with code ${code}`, {
    exitCode: code,
    signal: result.signal ?? undefined,
    stderr: result.stderr.slice(0, 1024),
  });
}

/** Map exec failures (abort / timeout / generic) to RuntimeError. Re-throws RuntimeError as-is. */
function rethrowExecFailure(cause: unknown, options: RunOptions, timeout: number): never {
  if (cause instanceof RuntimeError) throw cause;
  if (options.signal?.aborted || (cause instanceof Error && cause.name === "AbortError")) {
    throw new RuntimeError("CANCELLED", "ast-sgrep execution was cancelled");
  }
  const message = cause instanceof Error ? cause.message : String(cause);
  if (/timeout|timed out/i.test(message)) {
    throw new RuntimeError("TIMEOUT", `ast-sgrep exceeded ${timeout}ms`, { timeoutMs: timeout });
  }
  throw new RuntimeError("EXEC_FAILED", "Unable to execute ast-sgrep", { cause: message });
}

function parseEnvelope(result: ExecResult, limit: number): MachineEnvelope {
  const stdoutBytes = byteLength(result.stdout);
  const stderrBytes = byteLength(result.stderr);
  // Byte lengths are non-negative: sum > limit covers either-side overflow and combined cap.
  if (stdoutBytes + stderrBytes > limit) {
    throw new RuntimeError("OUTPUT_LIMIT", "ast-sgrep output exceeded the configured limit", { limit, stdoutBytes, stderrBytes });
  }
  const code = result.exitCode ?? result.code ?? 0;
  if (code !== 0) {
    throwNonzeroProcessFailure(result, code);
  }
  let value: unknown;
  try { value = JSON.parse(result.stdout); }
  catch (cause) { throw new RuntimeError("MALFORMED_OUTPUT", "ast-sgrep returned malformed JSON", { cause: cause instanceof Error ? cause.message : String(cause) }); }
  const envelope = record(value) as Partial<MachineEnvelope> | undefined;
  if (!envelope) throw new RuntimeError("MALFORMED_OUTPUT", "ast-sgrep returned a non-object JSON payload");
  // Protocol field varieties (Ashby Keep) — sequential wire-contract checks stay here.
  if (envelope.tool !== "asgrep") throw new RuntimeError("TOOL_MISMATCH", "Response is not from ast-sgrep", { actual: envelope.tool });
  if (envelope.schema_version !== MACHINE_SCHEMA_VERSION) throw new RuntimeError("PROTOCOL_MISMATCH", "Unsupported ast-sgrep machine protocol", { expected: MACHINE_SCHEMA_VERSION, actual: envelope.schema_version });
  if (typeof envelope.ok !== "boolean") throw new RuntimeError("MALFORMED_OUTPUT", "ast-sgrep response is missing boolean ok");
  if (!envelope.ok) {
    // Preserve pre-extract failure shape: plain object check (arrays allowed as error bag).
    const failure = envelope.error && typeof envelope.error === "object" ? envelope.error as Record<string, unknown> : undefined;
    const message = typeof failure?.message === "string" ? failure.message : "ast-sgrep reported an operational failure";
    throw new RuntimeError("OPERATIONAL_ERROR", message, { command: envelope.command, error: failure });
  }
  assertVersionTriple(envelope);
  return envelope as MachineEnvelope;
}

function throwIndexRebuildFailed(
  cause: unknown,
  indexPath: string,
  backupPath: string,
  priorMoved: boolean,
): never {
  let recoveryPath = indexPath;
  let priorIndexPreserved = existsSync(indexPath);
  if (priorMoved && !priorIndexPreserved && existsSync(backupPath)) {
    recoveryPath = backupPath;
    priorIndexPreserved = true;
  }
  throw new RuntimeError("INDEX_REBUILD_FAILED", "Incompatible index rebuild failed; the prior index remains recoverable", {
    indexPath,
    recoveryPath,
    priorIndexPreserved,
    expectedIndexFormat: INDEX_FORMAT_VERSION,
    cause: cause instanceof Error ? cause.message : String(cause),
  });
}


async run(args: readonly string[], context: RuntimeContext, options: RunOptions = {}): Promise<MachineEnvelope> {
    if (!Array.isArray(args) || args.some((arg) => typeof arg !== "string")) throw new RuntimeError("INVALID_ARGUMENTS", "Arguments must be a string array");
    if (options.signal?.aborted) throw new RuntimeError("CANCELLED", "ast-sgrep execution was cancelled");
    const root = await this.resolveRoot(context);
    const timeout = finitePositive(options.timeoutMs, this.config.timeoutMs, "timeoutMs");
    const env: NodeJS.ProcessEnv = { ...this.#environment, ...this.config.env, ...options.env, NO_COLOR: "1" };
    const binary = getBinary(this.config, env, this.#resolver);
    try {
      const execOptions: ExecOptions = { cwd: root, env, timeout };
      if (options.signal) execOptions.signal = options.signal;
      const result = await this.pi.exec(binary, Object.freeze([...args]), execOptions);
      return parseEnvelope(result, this.config.maxOutputBytes);
    } catch (cause) {
      rethrowExecFailure(cause, options, timeout);
    }
  }

  /** Absolute path to the native binary (for sticky serve / stdin batch spawn). */
  