import { type MachineEnvelope } from "./types.js";
export type IndexHealth = "ready" | "missing" | "incompatible";
export declare function pathContained(parent: string, child: string): boolean;
export declare function record(value: unknown): Record<string, unknown> | undefined;
export declare function indexHealth(status: MachineEnvelope): IndexHealth;
export declare function incompatibleStatusFailure(cause: unknown): boolean;
export declare function indexCompletion(response: MachineEnvelope, requireWalkErrors: boolean): {
    failed: number;
    walkErrors: boolean;
};
export declare function indexPathFor(root: string, env: NodeJS.ProcessEnv): string;
export declare function indexQuarantines(indexPath: string): string[];
/** Classify a rebuild failure and identify recovery copies made by this attempt. */
export declare function throwIndexRebuildFailed(cause: unknown, indexPath: string, quarantinesBefore: ReadonlySet<string>): never;
/** Read the on-disk index format marker. The binary is the authority on what it can read. */
export declare function inspectIndexFile(path: string): "missing" | "incompatible" | number;
