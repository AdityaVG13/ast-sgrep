import { createRequire } from "node:module";

export type SqliteBackend = "node" | "bun";

export interface IndexStatement {
  get(...params: unknown[]): unknown;
  run(...params: unknown[]): unknown;
}

export interface IndexDatabase {
  prepare(sql: string): IndexStatement;
  exec(sql: string): unknown;
  close(): void;
}

type SqliteModule = {
  DatabaseSync?: SqliteCtor;
  Database?: SqliteCtor;
};

type SqliteCtor = new (path: string, options?: Record<string, unknown>) => {
  prepare?(sql: string): IndexStatement;
  query?(sql: string): IndexStatement;
  exec(sql: string): unknown;
  close(): void;
};

type LoadedBackend = { backend: SqliteBackend; Ctor: SqliteCtor };

let cached: LoadedBackend | undefined;

function bunVersion(): string | undefined {
  return (process.versions as NodeJS.ProcessVersions & { bun?: string }).bun;
}

function loadModule(specifier: "node:sqlite" | "bun:sqlite"): SqliteModule {
  return createRequire(import.meta.url)(specifier) as SqliteModule;
}

function loadBackend(): LoadedBackend {
  if (cached) return cached;
  if (bunVersion() !== undefined) {
    cached = { backend: "bun", Ctor: requireCtor(loadModule("bun:sqlite"), "Database") };
    return cached;
  }
  try {
    cached = { backend: "node", Ctor: requireCtor(loadModule("node:sqlite"), "DatabaseSync") };
    return cached;
  } catch (nodeError) {
    try {
      cached = { backend: "bun", Ctor: requireCtor(loadModule("bun:sqlite"), "Database") };
      return cached;
    } catch {
      throw new Error("No SQLite backend available (node:sqlite and bun:sqlite both failed)", {
        cause: nodeError,
      });
    }
  }
}

function requireCtor(mod: SqliteModule, name: "DatabaseSync" | "Database"): SqliteCtor {
  const Ctor = mod[name];
  if (typeof Ctor !== "function") {
    throw new Error(`SQLite module is missing ${name}`);
  }
  return Ctor;
}

export function sqliteBackend(): SqliteBackend {
  return loadBackend().backend;
}

/** Open the index DB with Node `node:sqlite` or Bun `bun:sqlite`. */
export function openIndexDatabase(path: string, options: { readOnly?: boolean } = {}): IndexDatabase {
  const { backend, Ctor } = loadBackend();
  const readOnly = options.readOnly === true;
  const database = backend === "bun"
    ? new Ctor(path, { readonly: readOnly, create: !readOnly })
    : new Ctor(path, { readOnly });
  return {
    prepare(sql: string): IndexStatement {
      const statement = database.prepare?.(sql) ?? database.query?.(sql);
      if (!statement) throw new Error("SQLite statement API is unavailable");
      return statement;
    },
    exec(sql: string) {
      return database.exec(sql);
    },
    close() {
      database.close();
    },
  };
}
