/** Shared redis client. */
import { Redis } from "redis-client";

export const redis = new Redis({ host: "localhost", port: 6379 });
