// axios-family: module factory function + object-literal factory namespace.
export interface AxiosInstance {
  request(config: unknown): Promise<unknown>;
}

let defaultTimeout = 1000;

function createInstance(defaultConfig: unknown): AxiosInstance {
  return {
    request: (config: unknown) => Promise.resolve(config),
  };
}

export const axios = {
  create: (config: unknown) => createInstance(config),
  request: (config: unknown) => Promise.resolve(config),
};

export function createClient(baseURL: string): AxiosInstance {
  return createInstance({ baseURL });
}
