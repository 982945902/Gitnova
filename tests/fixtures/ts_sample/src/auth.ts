import { formatDate, logger } from "./utils";

export interface Session {
  userId: string;
  expiresAt: Date;
}

export function validateSession(session: Session): boolean {
  logger(`validating ${formatDate(session.expiresAt)}`);
  return session.userId.length > 0 && session.expiresAt > new Date();
}

