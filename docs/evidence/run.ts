import { readFileSync, rmSync, existsSync } from "node:fs"
let wins = 0, losses = 0, codes = new Set<string>(), mismatch = 0
for (let round = 0; round < 30; round++) {
  const target = `D:/claude-cache/lock-race/arena/r${round}.lock`
  const procs = Array.from({ length: 12 }, (_, i) => Bun.spawn(["bun", "racer.ts", target, `p${i}`], { stdout: "pipe", cwd: "D:/claude-cache/lock-race" }))
  const outs = await Promise.all(procs.map(async (p) => (await new Response(p.stdout).text()).trim()))
  const w = outs.filter((o) => o.startsWith("WIN"))
  wins += w.length; losses += outs.length - w.length
  for (const o of outs) if (o.startsWith("LOSE")) codes.add(o.split(" ")[2]!)
  if (w.length !== 1) { console.log(`round ${round}: ${w.length} winners!`, outs); mismatch++ }
  else if (readFileSync(target, "utf8") !== w[0]!.slice(4)) { console.log(`round ${round}: content mismatch`); mismatch++ }
}
console.log(`rounds=30 racers=12 wins=${wins} losses=${losses} loserCodes=${[...codes]} anomalies=${mismatch}`)
console.log("bun", Bun.version, "rename-over-existing:", (() => { try { Bun.write("arena/a.txt", "a"); Bun.write("arena/b.txt", "b"); require("node:fs").renameSync("arena/a.txt", "arena/b.txt"); return readFileSync("arena/b.txt","utf8") === "a" } catch (e: any) { return e.code } })())
