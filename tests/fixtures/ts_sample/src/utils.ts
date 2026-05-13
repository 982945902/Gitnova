export function formatDate(value: Date): string {
  return value.toISOString().slice(0, 10);
}

export function logger(message: string): void {
  console.log(message);
}

