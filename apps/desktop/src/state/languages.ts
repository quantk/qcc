// Which language a file is written in, judged by its name, for highlighting.

/** By the whole file name, for files whose name says more than an extension. */
const byName: Record<string, string> = {
  "cmakelists.txt": "cmake",
  dockerfile: "dockerfile",
  gnumakefile: "make",
  justfile: "make",
  makefile: "make",
  "flake.lock": "json",
  "cargo.lock": "toml",
};

const byExtension: Record<string, string> = {
  bash: "bash",
  c: "c",
  cc: "cpp",
  cjs: "javascript",
  cpp: "cpp",
  cs: "csharp",
  css: "css",
  cxx: "cpp",
  dart: "dart",
  diff: "diff",
  ex: "elixir",
  exs: "elixir",
  go: "go",
  gql: "graphql",
  graphql: "graphql",
  h: "c",
  hcl: "hcl",
  hpp: "cpp",
  hs: "haskell",
  htm: "html",
  html: "html",
  ini: "ini",
  java: "java",
  js: "javascript",
  json: "json",
  jsonc: "jsonc",
  jsx: "jsx",
  kt: "kotlin",
  kts: "kotlin",
  less: "less",
  lua: "lua",
  md: "markdown",
  mdx: "markdown",
  mjs: "javascript",
  mk: "make",
  mts: "typescript",
  cts: "typescript",
  nix: "nix",
  patch: "diff",
  php: "php",
  proto: "proto",
  ps1: "powershell",
  py: "python",
  pyi: "python",
  rb: "ruby",
  rs: "rust",
  scala: "scala",
  scss: "scss",
  sh: "bash",
  sql: "sql",
  svelte: "svelte",
  svg: "xml",
  swift: "swift",
  tf: "hcl",
  toml: "toml",
  ts: "typescript",
  tsx: "tsx",
  vue: "vue",
  wit: "wit",
  xml: "xml",
  yaml: "yaml",
  yml: "yaml",
  zig: "zig",
  zsh: "bash",
};

/** The highlighter's name for the language of `path`, or null when it is not known. */
export function languageOfPath(path: string): string | null {
  const name = (path.split("/").at(-1) ?? path).toLowerCase();
  if (name in byName) return byName[name]!;
  if (name.startsWith("dockerfile.") || name.endsWith(".dockerfile")) return "dockerfile";
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return null;
  return byExtension[name.slice(dot + 1)] ?? null;
}
