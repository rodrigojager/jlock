import { writeFileSync, renameSync, readFileSync, unlinkSync, mkdirSync } from "node:fs"
const out: string[] = []
writeFileSync("arena/a.txt", "a"); writeFileSync("arena/b.txt", "b")
try { renameSync("arena/a.txt", "arena/b.txt"); out.push(`bun rename-over-existing (closed files): ok, content=${readFileSync("arena/b.txt", "utf8")}`) } catch (e: any) { out.push(`bun rename-over-existing: ${e.code}`) }
try { mkdirSync("arena/dir.lock") } catch {}
try { mkdirSync("arena/dir.lock"); out.push("mkdir twice: no error") } catch (e: any) { out.push(`mkdir twice: ${e.code}`) }
writeFileSync("arena/held.txt", "held")
const holder = Bun.spawn(["bun", "holder.ts"], { stdout: "pipe", cwd: "D:/claude-cache/lock-race" })
await holder.stdout.getReader().read()
try { unlinkSync("arena/held.txt"); out.push("unlink while another process reads it: ok") } catch (e: any) { out.push(`unlink while another process reads it: ${e.code}`) }
try { renameSync("arena/held.txt", "arena/moved.txt"); out.push("rename it while another process reads it: ok") } catch (e: any) { out.push(`rename it while another process reads it: ${e.code}`) }
writeFileSync("arena/new.txt", "new")
try { renameSync("arena/new.txt", "arena/held.txt"); out.push("rename over it while another process reads it: ok") } catch (e: any) { out.push(`rename over it while another process reads it: ${e.code}`) }
await holder.exited
console.log(out.join("\n"))
