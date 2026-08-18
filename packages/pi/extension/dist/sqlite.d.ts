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
export declare function sqliteBackend(): SqliteBackend;
/** Open the index DB with Node `node:sqlite` or Bun `bun:sqlite`. */
export declare function openIndexDatabase(path: string, options?: {
    readOnly?: boolean;
}): IndexDatabase;
