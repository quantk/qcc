import os, random, subprocess, sys, json
root = sys.argv[1]
random.seed(7)
os.makedirs(root, exist_ok=True)
def sh(*a): subprocess.run(a, cwd=root, check=True, stdout=subprocess.DEVNULL)
sh("git", "init", "-q", "-b", "main")
sh("git", "config", "user.email", "bench@example.com"); sh("git", "config", "user.name", "bench")
def ts_file(n, seed):
    r = random.Random(seed)
    out = []
    for i in range(n):
        k = r.randrange(4)
        if k == 0: out.append(f"export function fn{i}(a: number, b: string): string {{ return `${{a}}-${{b}}`; }}")
        elif k == 1: out.append(f"const value{i} = {{ name: \"item {i}\", count: {r.randrange(1000)}, tags: [\"x\", \"y\"] }};")
        elif k == 2: out.append(f"// A comment about line {i} that explains something rather long and wordy.")
        else: out.append(f"if (value{max(0,i-1)}.count > {r.randrange(100)}) console.log(\"line {i}\");")
    return "\n".join(out) + "\n"
def rs_file(n, seed):
    r = random.Random(seed)
    return "\n".join(f"pub fn f{i}(x: u32) -> u32 {{ x.wrapping_mul({r.randrange(99)}) + {i} }}" for i in range(n)) + "\n"
exts = [("ts", ts_file), ("rs", rs_file), ("py", lambda n, s: "\n".join(f"def f{i}(x):\n    return x * {i}" for i in range(n)) + "\n")]
paths = []
for d in range(20):
    for f in range(20):
        ext, gen = exts[(d + f) % 3]
        p = f"src/mod{d}/file{f}.{ext}"
        os.makedirs(os.path.dirname(os.path.join(root, p)), exist_ok=True)
        open(os.path.join(root, p), "w").write(gen(200, d * 100 + f)); paths.append((p, gen))
# a big file and a lockfile
open(os.path.join(root, "src/big.ts"), "w").write(ts_file(8000, 1))
lock = {"name": "x", "lockfileVersion": 3, "packages": {f"node_modules/pkg{i}": {"version": f"1.{i}.0", "resolved": f"https://registry.npmjs.org/pkg{i}/-/pkg{i}-1.{i}.0.tgz", "integrity": f"sha512-{'a'*60}{i}"} for i in range(6000)}}
open(os.path.join(root, "package-lock.json"), "w").write(json.dumps(lock, indent=2))
os.makedirs(os.path.join(root, "assets"), exist_ok=True)
for i in range(10): open(os.path.join(root, f"assets/img{i}.png"), "wb").write(os.urandom(20000))
sh("git", "add", "-A"); sh("git", "commit", "-q", "-m", "base")
# changes: modify 300 files
r = random.Random(3)
for p, gen in paths[:300]:
    full = os.path.join(root, p); lines = open(full).read().split("\n")
    for _ in range(15):
        i = r.randrange(len(lines)); lines[i] = lines[i] + " // changed " + str(r.randrange(10**6))
    for _ in range(5): lines.insert(r.randrange(len(lines)), "// inserted line " + str(r.randrange(10**6)))
    open(full, "w").write("\n".join(lines))
for p, _ in paths[300:320]: os.remove(os.path.join(root, p))
open(os.path.join(root, "src/big.ts"), "w").write(ts_file(8000, 2))
lock["packages"] = {k.replace("pkg", "dep"): {**v, "version": v["version"] + "-1"} for k, v in lock["packages"].items()}
open(os.path.join(root, "package-lock.json"), "w").write(json.dumps(lock, indent=2))
for i in range(5): open(os.path.join(root, f"assets/img{i}.png"), "wb").write(os.urandom(20000))
os.makedirs(os.path.join(root, "new"), exist_ok=True)
for i in range(60): open(os.path.join(root, f"new/added{i}.ts"), "w").write(ts_file(150, 500 + i))
