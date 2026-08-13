/** Contract-ontology ts half: an interface implementation (extension
 * contract) and a serializer/deserializer pair (serialization contract). */
import { Plugin } from "./contracts";

export class EchoPlugin implements Plugin {
  run(): void {
    /* no-op */
  }
  name(): string {
    return "echo";
  }
}

export function toJson(value: unknown): string {
  return JSON.stringify(value);
}

export function fromJson(text: string): unknown {
  return JSON.parse(text);
}
