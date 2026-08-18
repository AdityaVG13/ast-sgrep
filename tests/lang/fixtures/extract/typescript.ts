/** Fixture docs mention docOnlyTypeScript and should not become code. */
import { WidgetSource } from "lib/widgets";

type WidgetName = string;

/** Function docs mention docOnlyTypeScript. */
export function makeWidget(source: WidgetSource): WidgetName {
  return source.name;
}

export class GoldenWidget {
  /** Method docs mention docOnlyTypeScript. */
  render(source: WidgetSource): string {
    return formatWidget(makeWidget(source));
  }
}

/** Arrow function docs mention docOnlyTypeScript. */
export const formatWidget = (name: WidgetName): string => name.trim();

/** Interface docs mention docOnlyTypeScript. */
export interface WidgetSourceLike {
  name: string;
}

/** Enum docs mention docOnlyTypeScript. */
export enum WidgetState {
  Ready,
  Spent,
}
