// One racer: tries to create the same file exclusively, reports win/lose and the error code.
import { openSync, closeSync, writeSync } from "node:fs"
const [target, id] = [process.argv[2]!, process.argv[3]!]
const start = Bun.nanoseconds()
while (Bun.nanoseconds() - start < 200_000_000) {} // spin so every racer attempts at about the same moment
try {
  const fd = openSync(target, "wx")
  writeSync(fd, id)
  closeSync(fd)
  console.log(`WIN ${id}`)
} catch (error: any) {
  console.log(`LOSE ${id} ${error.code}`)
}
