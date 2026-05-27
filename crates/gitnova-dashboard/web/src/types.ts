export type NodeKind =
  | "repository"
  | "file"
  | "module"
  | "function"
  | "method"
  | "class"
  | "struct"
  | "enum"
  | "union"
  | "typedef"
  | "variable"
  | "macro"
  | "trait"
  | "interface"
  | "import"
  | "unknown";

export type Language = "rust" | "typescript" | "javascript" | "python" | "cpp";

export interface Span {
  start_line: number;
  start_col: number;
  end_line: number;
  end_col: number;
}

export interface NodeMetrics {
  in_degree: number;
  out_degree: number;
  churn_90d: number;
  last_changed_unix: number | null;
  is_test: boolean;
}

export interface CodeNode {
  id: string;
  kind: NodeKind;
  name: string;
  qualified_name: string;
  path: string;
  span: Span | null;
  language: Language | null;
  text: string;
  tags: string[];
  metrics: NodeMetrics;
}

export interface CodeEdge {
  from: string;
  to: string;
  kind: string;
  confidence_basis_points: number;
}
