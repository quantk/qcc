import { describe, expect, it } from "vitest";

import { languageOfPath } from "./languages";

describe("languageOfPath", () => {
  it("judges by the extension, in any case", () => {
    expect(languageOfPath("crates/qcc-store/src/lib.rs")).toBe("rust");
    expect(languageOfPath("src/App.TSX")).toBe("tsx");
    expect(languageOfPath("a.b/config.yml")).toBe("yaml");
  });

  it("knows files by their whole name", () => {
    expect(languageOfPath("Makefile")).toBe("make");
    expect(languageOfPath("docker/Dockerfile.dev")).toBe("dockerfile");
    expect(languageOfPath("Cargo.lock")).toBe("toml");
  });

  it("gives up on what it does not know", () => {
    expect(languageOfPath("LICENSE")).toBeNull();
    expect(languageOfPath(".gitignore")).toBeNull();
    expect(languageOfPath("data.bin")).toBeNull();
    expect(languageOfPath("")).toBeNull();
  });
});
