// vue-family: `createApp` module factory; zod-family: `z` object-literal
// schema-builder namespace; guava-family java-style static factories are in
// GuavaStyle.java.

export interface App<HostElement = unknown> {
  mount(selector: string): void;
}

export const createApp = (rootComponent: unknown): App => ({
  mount: (_selector: string) => {},
});

export const z = {
  object: (shape: unknown) => ({ _shape: shape }),
  string: () => ({ _type: "string" }),
  number: () => ({ _type: "number" }),
  optional: (schema: unknown) => ({ _inner: schema }),
};
