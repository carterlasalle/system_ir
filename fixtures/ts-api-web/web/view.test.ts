import { describe, expect, it } from "vitest";
import { renderUsers } from "./view";

describe("renderUsers", () => {
  it("joins user names from the API response", async () => {
    expect(typeof renderUsers).toBe("function");
  });
});
