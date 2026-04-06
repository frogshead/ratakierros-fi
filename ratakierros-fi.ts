const PUBLIC_DIR = new URL("./public/", import.meta.url).pathname;

Deno.serve({ port: 8000 }, async (_request: Request) => {
  try {
    const html = await Deno.readTextFile(`${PUBLIC_DIR}index.html`);
    return new Response(html, {
      headers: { "content-type": "text/html; charset=utf-8" },
    });
  } catch {
    return new Response("Not found", { status: 404 });
  }
});

console.log("Frontend server running on http://0.0.0.0:8000");
