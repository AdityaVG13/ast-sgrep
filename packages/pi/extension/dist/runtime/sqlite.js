import { createRequire } from "node:module";
let cached;
function bunVersion() {
    return process.versions.bun;
}
function loadModule(specifier) {
    return createRequire(import.meta.url)(specifier);
}
function loadBackend() {
    if (cached)
        return cached;
    if (bunVersion() !== undefined) {
        cached = { backend: "bun", Ctor: requireCtor(loadModule("bun:sqlite"), "Database") };
        return cached;
    }
    try {
        cached = { backend: "node", Ctor: requireCtor(loadModule("node:sqlite"), "DatabaseSync") };
        return cached;
    }
    catch (nodeError) {
        try {
            cached = { backend: "bun", Ctor: requireCtor(loadModule("bun:sqlite"), "Database") };
            return cached;
        }
        catch {
            throw new Error("No SQLite backend available (node:sqlite and bun:sqlite both failed)", {
                cause: nodeError,
            });
        }
    }
}
function requireCtor(mod, name) {
    const Ctor = mod[name];
    if (typeof Ctor !== "function") {
        throw new Error(`SQLite module is missing ${name}`);
    }
    return Ctor;
}
export function sqliteBackend() {
    return loadBackend().backend;
}
/** Open the index DB with Node `node:sqlite` or Bun `bun:sqlite`. */
export function openIndexDatabase(path, options = {}) {
    const { backend, Ctor } = loadBackend();
    const readOnly = options.readOnly === true;
    const database = backend === "bun"
        ? new Ctor(path, { readonly: readOnly, create: !readOnly })
        : new Ctor(path, { readOnly });
    return {
        prepare(sql) {
            const statement = database.prepare?.(sql) ?? database.query?.(sql);
            if (!statement)
                throw new Error("SQLite statement API is unavailable");
            return statement;
        },
        exec(sql) {
            return database.exec(sql);
        },
        close() {
            database.close();
        },
    };
}
