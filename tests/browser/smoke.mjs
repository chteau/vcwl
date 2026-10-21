import puppeteer from "puppeteer-core";
import { serve } from "./serve.mjs";
const { server, url } = await serve(process.argv[2]);
const browser = await puppeteer.launch({ executablePath: "/usr/bin/google-chrome", headless: true, args: ["--no-sandbox"] });
const page = await browser.newPage();
page.on("console", (m) => console.log("console:", m.type(), m.text()));
page.on("pageerror", (e) => console.log("pageerror:", e.message));
await page.goto(url);
await page.waitForFunction(() => window.vcwl, { timeout: 5000 }).catch(() => {});
console.log(await page.evaluate(() => document.body.innerHTML));
const btn = await page.$("button");
if (btn) { await btn.click(); await btn.click(); }
console.log(await page.evaluate(() => document.body.innerText));
await browser.close();
server.close();
