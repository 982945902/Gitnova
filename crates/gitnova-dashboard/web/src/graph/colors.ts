import type { NodeKind } from "../types";

export function colorForKind(kind: NodeKind): number {
  switch (kind) {
    case "file":
      return 0x4ecdc4;
    case "module":
      return 0xa78bfa;
    case "class":
    case "struct":
    case "interface":
    case "trait":
      return 0xf5b85f;
    case "method":
      return 0xe879b9;
    case "function":
      return 0x7aa2ff;
    case "enum":
    case "union":
    case "typedef":
      return 0x8ee58b;
    default:
      return 0x94a3b8;
  }
}

export function edgeColor(kind: string, highlighted: boolean): number {
  if (highlighted) {
    return 0xff5d7a;
  }
  return kind === "calls" ? 0x6fa4ff : 0x657389;
}
