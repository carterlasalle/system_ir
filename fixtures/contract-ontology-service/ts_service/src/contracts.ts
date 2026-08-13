/** Contract-ontology ts half: the extension point interface. Declared in
 * its own file so implementing classes in other files register against a
 * non-local surface (the extension contract). */
export interface Plugin {
  run(): void;
  name(): string;
}
