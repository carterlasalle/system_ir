/** Shared database client. */
export const db = {
  users: {
    findMany: async (opts?: unknown) => [],
    create: async (data: { data: unknown }) => data.data,
  },
};
