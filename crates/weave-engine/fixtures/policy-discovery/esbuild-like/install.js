// Fixture only — discovery reads this; Weave must never execute it in Phase 9 tests.
const fs = require("fs");
fs.mkdirSync("bin", { recursive: true });
fs.writeFileSync("bin/esbuild", "#!/bin/sh\necho fixture\n");
