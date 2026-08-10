export type EditReplacePlan = {
    mode: "replace";
    absolutePath: string;
    displayPath: string;
    oldString: string;
    newString: string;
    replaceAll: boolean;
};
export type EditWritePlan = {
    mode: "write";
    absolutePath: string;
    displayPath: string;
    contents: string;
};
export type EditPlan = EditReplacePlan | EditWritePlan;
/** Wire/tool replace mode: old_string+new_string required; contents unrepresentable. */
export type EditReplaceParams = {
    path: string;
    old_string: string;
    new_string: string;
    replace_all?: boolean;
};
/** Wire/tool write mode: full contents; replace fields unrepresentable. */
export type EditWriteParams = {
    path: string;
    contents: string;
};
/** Replace XOR write -- illegal both/neither cannot be typed after boundary parse. */
export type EditParams = EditReplaceParams | EditWriteParams;
export type EditResult = {
    path: string;
    mode: "replace" | "write";
    replacements?: number;
    created?: boolean;
};
/** Max chars kept per line when scanning text for binary/UTF-8 checks (minified-line defense). */
export declare const MAX_EDIT_LINE_CHARS = 2000;
/** Soft byte ceiling for replace targets before we require write mode. */
export declare const MAX_EDIT_REPLACE_BYTES: number;
/** Refuse device / proc fd paths before any I/O (cwd=/ is not a workspace boundary). */
export declare function assertSafeEditTarget(absolutePath: string): void;
/** Repair common model path mistakes before resolve (validate-then-repair). */
export declare function repairEditPath(raw: string): string;
/**
 * Boundary-parse untrusted tool args into EditParams.
 * Replace XOR write is enforced here so illegal bags never enter planEdit as trusted input.
 */
export declare function parseEditParams(params: unknown): EditParams;
/** Parse tool params into a root-bounded EditPlan (trusted after parseEditParams). */
export declare function planEdit(params: EditParams, projectRoot: string): EditPlan;
/** Apply a planned edit; returns structured result for the tool details. */
export declare function applyEdit(plan: EditPlan): Promise<EditResult>;
