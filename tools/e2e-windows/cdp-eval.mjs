// Debugging aid while writing scenarios: evaluate JS in one of the app's
// pages through an open CDP tunnel (lib/ports.mjs CDP_PORT).
//   node tools/e2e-windows/cdp-eval.mjs settings.html 'document.title'
//   node tools/e2e-windows/cdp-eval.mjs settings.html --shot out.png
import { Page, listPages, waitPage } from "./lib/cdp.mjs";

const [part, expr, path] = process.argv.slice(2);
if (!part) {
  for (const p of await listPages()) console.log(p.url);
  process.exit(0);
}
const page = await Page.open(await waitPage(part, { timeoutMs: 5000 }));
try {
  if (expr === "--shot") console.log(await page.shot(path));
  else console.log(JSON.stringify(await page.eval(expr), null, 1));
} finally {
  page.close();
}
