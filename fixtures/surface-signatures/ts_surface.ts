export interface Repo<T> {
  find(id: string): Promise<T | null>;
}

export class IncidentRepo implements Repo<Incident> {
  async findByOwner(
    owner: string,
    opts?: { limit?: number },
  ): Promise<Incident[]> {
    return [];
  }
}

export type Incident = {
  id: string;
};
