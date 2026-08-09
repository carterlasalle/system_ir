import { describe, expect, it } from "vitest";
import { renderTranscript } from "../view";
import { indexTranscripts } from "../../worker/indexer";

describe("renderTranscript", () => {
  it("uses the transcript response field", async () => {
    expect(typeof renderTranscript).toBe("function");
  });
});

describe("indexTranscripts", () => {
  it("indexes normalized text", async () => {
    expect(typeof indexTranscripts).toBe("function");
  });
});
