import { formatDate } from "./utils";

export function invoiceLabel(date: Date): string {
  return `Invoice ${formatDate(date)}`;
}

