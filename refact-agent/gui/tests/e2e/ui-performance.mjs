import { chromium } from "@playwright/test";

const browser = await chromium.launch({ headless: true });
const results = [];
try {
  for (const count of [10, 30, 60, 100]) {
    const page = await browser.newPage({
      viewport: { width: 1440, height: 1000 },
    });
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    await page.goto(
      `http://127.0.0.1:4187/tests/e2e/route-showcase.html?route=ui-performance&chats=${count}`,
    );
    const input = page.locator("textarea");
    await input.waitFor();
    await page.waitForFunction(
      () => document.querySelectorAll('[role="tab"]').length >= 2,
    );
    await input.fill("draft preserved ");
    const { initial, ...measurement } = await page.evaluate(async (chats) => {
      const fixture = window.__uiPerformance;
      const input = document.querySelector("textarea");
      const samples = [];
      const initialRenders = window.__workspaceAppRenders;
      const initial = Array.from({ length: chats - 1 }, (_, index) => {
        const id = `perf-${index + 1}`;
        return fixture.store.getState().chat.threads[id].thread.messages[0]
          .content;
      });
      for (let i = 0; i < 60; i++) {
        const start = performance.now();
        for (let chat = 1; chat < chats; chat++)
          fixture.delta(`perf-${chat}`, `${i},`);
        await new Promise(requestAnimationFrame);
        samples.push(performance.now() - start);
      }
      const suffix = Array.from({ length: 60 }, (_, i) => `${i},`).join("");
      for (let chat = 1; chat < chats; chat++) {
        const id = `perf-${chat}`;
        const content =
          fixture.store.getState().chat.threads[id].thread.messages[0].content;
        if (content !== initial[chat - 1] + suffix)
          throw new Error(`Ordered delta loss: ${id}`);
      }
      samples.sort((a, b) => a - b);
      return {
        deltaFrameP50: samples[30],
        deltaFrameP95: samples[57],
        rootRenders: window.__workspaceAppRenders - initialRenders,
        ordered: true,
        initial,
        draft: input.value,
      };
    }, count);
    const typing = [];
    for (let i = 0; i < 20; i++) {
      await page.evaluate((chats) => {
        for (let chat = 1; chat < chats; chat++)
          window.__uiPerformance.delta(`perf-${chat}`, "typing,");
      }, count);
      const start = performance.now();
      await input.press("x");
      await page.evaluate(() => new Promise(requestAnimationFrame));
      typing.push(performance.now() - start);
    }
    if ((await input.inputValue()) !== "draft preserved " + "x".repeat(20))
      throw new Error(
        `Draft changed: ${JSON.stringify(await input.inputValue())}`,
      );
    if (!(await input.evaluate((node) => node === document.activeElement)))
      throw new Error("Composer lost focus");
    typing.sort((a, b) => a - b);
    const tab = page
      .getByRole("tab", { name: /Performance chat perf-1(?:\s|$)/ })
      .first();
    const switchStart = performance.now();
    await tab.click();
    await page.waitForFunction(
      () =>
        window.__uiPerformance.store.getState().chat.current_thread_id ===
        "perf-1",
    );
    const switchMs = performance.now() - switchStart;
    await page.evaluate(
      ({ chats, initial }) => {
        const suffix =
          Array.from({ length: 60 }, (_, i) => `${i},`).join("") +
          "typing,".repeat(20);
        for (let chat = 1; chat < chats; chat++) {
          const id = `perf-${chat}`;
          const content =
            window.__uiPerformance.store.getState().chat.threads[id].thread
              .messages[0].content;
          if (content !== initial[chat - 1] + suffix)
            throw new Error(`Ordered typing delta loss: ${id}`);
        }
      },
      { chats: count, initial },
    );
    if (errors.length !== 0)
      throw new Error(`Browser errors: ${JSON.stringify(errors)}`);
    results.push({
      count,
      ...measurement,
      typingP50: typing[10],
      typingP95: typing[19],
      switchMs,
      errors,
    });
    await page.close();
  }
  console.log(
    JSON.stringify(
      { label: process.argv[2] ?? "unspecified", results },
      null,
      2,
    ),
  );
} finally {
  await browser.close();
}
