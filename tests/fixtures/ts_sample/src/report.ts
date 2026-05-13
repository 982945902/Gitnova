import { formatDate } from "./utils";

export function renderReport(createdAt: Date): string {
  return `Report ${formatDate(createdAt)}`;
}

