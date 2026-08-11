const fs = require("fs");

// Declared output: generated/hello.txt (see ADR-0018 / Phase 7 fixture).
fs.mkdirSync("generated", { recursive: true });
fs.writeFileSync("generated/hello.txt", "weave-exec-ok\n");
