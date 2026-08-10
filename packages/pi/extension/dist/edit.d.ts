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
export type EditParams = {
    path: string;
    old_string?: string;
    new_string?: string;
    contents?: string;
    replace_all?: boolean;
};
export type EditResult = {
    path: string;
    mode: "replace" | "write";
    replacements?: number;
    created?: boolean;
};
/** Parse tool params into a root-bounded EditPlan. */
export declare function planEdit(params: EditParams, projectRoot: string): EditPlan;
/** Apply a planned edit; returns structured result for the tool details. */
export declare function applyEdit(plan: EditPlan): Promise<EditResult>;
