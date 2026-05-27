import type { CodeNode } from "../types";

export interface GraphPoint {
  x: number;
  y: number;
  z: number;
}

const degree = (node: CodeNode) =>
  (node.metrics?.in_degree ?? 0) + (node.metrics?.out_degree ?? 0);

export function layoutGraph(nodes: CodeNode[]): Map<string, GraphPoint> {
  const sorted = [...nodes].sort(
    (left, right) => degree(right) - degree(left) || left.id.localeCompare(right.id),
  );
  const positions = new Map<string, GraphPoint>();
  const count = Math.max(1, sorted.length);
  const baseRadius = Math.max(140, Math.min(520, count * 16));

  sorted.forEach((node, index) => {
    const angle = (Math.PI * 2 * index) / count;
    const hubPull = Math.min(0.56, degree(node) / 30);
    const ringOffset = (index % 5) * 18;
    const radius = baseRadius * (1 - hubPull) + ringOffset;
    const pathBucket = stableBucket(node.path || node.qualified_name, 7) - 3;
    const kindBucket = stableBucket(node.kind, 5) - 2;
    positions.set(node.id, {
      x: Math.cos(angle) * radius,
      y: Math.sin(angle) * radius * 0.72,
      z: pathBucket * 42 + kindBucket * 18 + (index % 3) * 8,
    });
  });

  return positions;
}

export function nodeDegree(node: CodeNode): number {
  return degree(node);
}

function stableBucket(value: string, buckets: number): number {
  let hash = 0;
  for (let index = 0; index < value.length; index += 1) {
    hash = (hash * 31 + value.charCodeAt(index)) >>> 0;
  }
  return hash % buckets;
}
