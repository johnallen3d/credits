// Spike helper: forwards to the credits server and adds the CORS header the
// web host needs. Native hosts (iOS, macOS, Linux) don't need it.
//   bun tools/cors-proxy.ts [upstream] [port]
const upstream = process.argv[2] ?? 'http://192.168.4.26:3000';
const port = Number(process.argv[3] ?? 8790);

Bun.serve({
  hostname: '127.0.0.1',
  port,
  async fetch(req) {
    const url = new URL(req.url);
    const cors = { 'access-control-allow-origin': '*', 'access-control-allow-headers': '*' };
    if (req.method === 'OPTIONS') return new Response(null, { headers: cors });
    const res = await fetch(upstream + url.pathname + url.search, {
      method: req.method,
      headers: { accept: 'application/json' },
      body: req.method === 'GET' ? undefined : await req.arrayBuffer(),
    });
    const headers = new Headers(res.headers);
    for (const [k, v] of Object.entries(cors)) headers.set(k, v);
    headers.delete('content-encoding');
    headers.delete('content-length');
    return new Response(await res.arrayBuffer(), { status: res.status, headers });
  },
});
console.log(`cors-proxy: http://127.0.0.1:${port} -> ${upstream}`);
