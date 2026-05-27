import { describe, expect, it } from "vitest";
import { GRAPH_MODES, modeConfig } from "./modes";

describe("graph modes", () => {
  it("defines analyze and showcase modes", () => {
    expect(GRAPH_MODES).toEqual(["analyze", "showcase"]);
    expect(modeConfig.analyze.autoRotate).toBe(false);
    expect(modeConfig.showcase.autoRotate).toBe(true);
  });
});
