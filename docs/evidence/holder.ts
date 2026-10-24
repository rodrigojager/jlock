import { openSync, closeSync } from "node:fs"
const fd = openSync("D:/claude-cache/lock-race/arena/held.txt", "r")
process.stdout.write("open\n")
setTimeout(() => closeSync(fd), 3000)
