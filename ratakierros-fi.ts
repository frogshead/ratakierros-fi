const PUBLIC_DIR = new URL("./public/", import.meta.url).pathname;
const API_BASE = Deno.env.get("API_BASE") ?? "http://localhost:3000";

Deno.serve({ port: 8000 }, async (_request: Request) => {
  try {
    let html = await Deno.readTextFile(`${PUBLIC_DIR}index.html`);
    html = html.replace(
      "<!-- API_BASE_PLACEHOLDER -->",
      `<script>window.API_BASE = ${JSON.stringify(API_BASE)};</script>`,
    );
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  } catch {
    return new Response("Not found", { status: 404 });
  }
});

console.log(`Frontend server running on http://0.0.0.0:8000 (API: ${API_BASE})`);
