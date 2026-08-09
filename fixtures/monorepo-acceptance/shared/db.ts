/** Shared database clients: Postgres (prisma) + Redis. */
import { PrismaClient } from "@prisma/client";
import { Redis } from "redis-client";

export const prisma = new PrismaClient();
export const redis = new Redis();
