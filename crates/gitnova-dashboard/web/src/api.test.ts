import { describe, expect, it } from "vitest";
import { paths } from "./api";

describe("dashboard api paths", () => {
  it("encodes rank query", () => {
    expect(paths.rank("Where is auth session validated?", 10)).toBe(
      "/api/rank?query=Where%20is%20auth%20session%20validated%3F&limit=10",
    );
  });

  it("encodes graph context node id", () => {
    expect(paths.graphContext("gn_a/b:c", 1, 40)).toBe(
      "/api/graph-context?node_id=gn_a%2Fb%3Ac&depth=1&limit=40",
    );
  });
});
