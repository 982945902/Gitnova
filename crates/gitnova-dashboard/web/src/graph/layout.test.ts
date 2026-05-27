import { describe, expect, it } from "vitest";
import { layoutGraph } from "./layout";
import type { CodeNode } from "../types";

const node = (id: string, inDegree: number, outDegree: number): CodeNode => ({
  id,
  kind: "function",
  name: id,
  qualified_name: id,
  path: `${id}.ts`,
  span: null,
  language: "typescript",
  text: "",
  tags: [],
  metrics: {
    in_degree: inDegree,
    out_degree: outDegree,
    churn_90d: 0,
    last_changed_unix: null,
    is_test: false,
  },
});

describe("layoutGraph", () => {
  it("returns deterministic finite positions", () => {
    const nodes = [node("a", 1, 1), node("b", 1, 0), node("c", 4, 4)];
    const first = layoutGraph(nodes);
    const second = layoutGraph(nodes);

    expect(first).toEqual(second);
    expect(Number.isFinite(first.get("a")?.x)).toBe(true);
    expect(Number.isFinite(first.get("b")?.y)).toBe(true);
  });

  it("adds visible z-depth for multi-node graphs", () => {
    const positions = layoutGraph([node("a", 1, 1), node("b", 1, 0), node("c", 4, 4)]);
    const depths = [...positions.values()].map((point) => point.z);

    expect(new Set(depths).size).toBeGreaterThan(1);
    expect(Math.max(...depths) - Math.min(...depths)).toBeGreaterThan(20);
  });
});
