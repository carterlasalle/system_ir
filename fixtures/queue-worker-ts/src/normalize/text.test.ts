import { describe, expect, it } from "vitest";
import { normalize } from "../src/normalize/text";
import { resolveStreetName } from "../src/geo/resolver";

describe("normalize", () => {
  it("preserves raw text on resolver failure", () => {
    expect(normalize("unknown street")).toBe("unknown street");
  });
});

describe("resolveStreetName", () => {
  it("expands department street names", () => {
    expect(resolveStreetName("turn at Oak St")).toContain("Dept. A");
  });
});
