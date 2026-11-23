import { describe, expect, it } from "vitest";

import { appendQuote, quoteMarkdown } from "./quote";

describe("quoteMarkdown", () => {
  it("puts every line behind a quote mark", () => {
    expect(quoteMarkdown("one\ntwo")).toBe("> one\n> two");
  });

  it("keeps blank lines inside the quote as bare marks", () => {
    expect(quoteMarkdown("one\n\ntwo")).toBe("> one\n>\n> two");
  });

  it("drops blank lines at the edges and trailing spaces", () => {
    expect(quoteMarkdown("\n\n  one  \ntwo\t\n\n")).toBe(">   one\n> two");
  });

  it("reads Windows and old Mac line ends", () => {
    expect(quoteMarkdown("one\r\ntwo\rthree")).toBe("> one\n> two\n> three");
  });

  it("quotes nothing for whitespace", () => {
    expect(quoteMarkdown(" \n\t\n")).toBe("");
  });
});

describe("appendQuote", () => {
  it("fills an empty composer with the quote and a blank line after it", () => {
    expect(appendQuote("", "hello")).toBe("> hello\n\n");
  });

  it("adds the quote after typed text, a blank line apart", () => {
    expect(appendQuote("my question", "hello")).toBe("my question\n\n> hello\n\n");
  });

  it("does not stack blank lines after typed text that ends with some", () => {
    expect(appendQuote("my question\n\n\n", "hello")).toBe("my question\n\n> hello\n\n");
  });

  it("follows an earlier quote", () => {
    expect(appendQuote(appendQuote("", "one"), "two")).toBe("> one\n\n> two\n\n");
  });

  it("treats a composer of whitespace as empty", () => {
    expect(appendQuote("  \n", "hello")).toBe("> hello\n\n");
  });

  it("leaves the text alone for a blank selection", () => {
    expect(appendQuote("typed", "  \n ")).toBe("typed");
  });
});
