export const GRAPH_MODES = ["analyze", "showcase"] as const;

export type GraphMode = (typeof GRAPH_MODES)[number];

export const modeConfig = {
  analyze: {
    autoRotate: false,
    particles: false,
    depthScale: 1,
  },
  showcase: {
    autoRotate: true,
    particles: true,
    depthScale: 1.45,
  },
} satisfies Record<
  GraphMode,
  {
    autoRotate: boolean;
    particles: boolean;
    depthScale: number;
  }
>;
