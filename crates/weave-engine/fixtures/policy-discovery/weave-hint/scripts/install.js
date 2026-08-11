const fs = require("fs");
fs.mkdirSync("generated", { recursive: true });
fs.writeFileSync("generated/out.txt", "ok\n");
