// A dependency-free Node server with NO committed lockfile. Exercises the
// npm-install fallback (npm ci would fail with no package-lock.json) and the
// optional-lockfile COPY.
const http = require("http");
const port = process.env.PORT || 3000;
http
  .createServer((_req, res) => res.end("ok\n"))
  .listen(port, "0.0.0.0", () => console.log(`listening on 0.0.0.0:${port}`));
