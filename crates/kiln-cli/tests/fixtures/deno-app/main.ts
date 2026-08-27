// The provider's fallback start is `deno run --allow-net main.ts` (no
// --allow-env), so bind a fixed port rather than reading PORT.
Deno.serve({ port: 8000, hostname: "0.0.0.0" }, () => new Response("ok\n"));
