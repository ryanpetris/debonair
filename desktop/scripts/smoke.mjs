// Launches the built app against the host adapter simulator, walks through the views, checks that
// captured keys and key combinations reach the simulated Target, and saves screenshots. Run under a
// display, e.g. xvfb-run, after building the app and `cargo build -p debonair-sim`.
import { _electron as electron } from "playwright";
import { mkdirSync, mkdtempSync, readFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

const out = process.argv[2] ?? "out/screenshots";
const scheme = process.argv[3] ?? "light";
const simulator = resolve(process.env.DEBONAIR_SIMULATOR ?? "../rust/target/debug/debonair-sim");
mkdirSync(out, { recursive: true });
// A separate profile and simulator state, so nothing from a real installation is touched.
const profile = mkdtempSync(join(tmpdir(), "debonair-smoke-"));
const log = join(profile, "reports.log");
const app = await electron.launch({
  args: [".", `--user-data-dir=${profile}`, `--force-dark-mode=${scheme === "dark"}`],
  env: {
    ...process.env,
    DEBONAIR_SIMULATOR: simulator,
    DEBONAIR_SIM_LOG: log,
    DEBONAIR_SIM_STATE: join(profile, "adapter"),
    ELECTRON_ENABLE_LOGGING: "1",
  },
});
const errors = [];
const page = await app.firstWindow();
page.on("console", (m) => m.type() === "error" && errors.push(m.text()));
page.on("pageerror", (e) => errors.push(String(e)));
await page.emulateMedia({ colorScheme: scheme });
await app.evaluate(({ BrowserWindow }) => BrowserWindow.getAllWindows()[0]?.setContentSize(1040, 720));
const shot = async (name) => {
  await page.waitForTimeout(300);
  await page.screenshot({ path: `${out}/${scheme}-${name}.png` });
};
const reports = () => {
  try {
    return readFileSync(log, "utf8");
  } catch {
    return "";
  }
};
const expectReport = async (pattern, what) => {
  for (let i = 0; i < 40; i++) {
    if (pattern.test(reports())) return;
    await page.waitForTimeout(50);
  }
  throw new Error(`The Target did not receive ${what}.\n${reports()}`);
};

// With nothing paired, the adapter starts pairing and the simulated Target pairs by itself.
await page.getByText("Waiting to Pair").waitFor();
await shot("pairing");
await page.getByText("Click to Capture").waitFor({ timeout: 10000 });
await page.getByText(/Connected to Device 1/).waitFor();
await shot("control");

// Key combination buttons work without capturing.
await page.getByRole("button", { name: "Ctrl+Alt+Delete" }).click();
await expectReport(/report input1 05 00 4c/, "Ctrl+Alt+Delete");

// Capture, type, then release with Ctrl+Alt.
await page.getByText("Click to Capture").click();
await page.getByText("Capturing").waitFor();
await shot("capturing");
await page.keyboard.down("Shift");
await page.keyboard.press("KeyA");
await page.keyboard.up("Shift");
await expectReport(/report input1 02 00 04/, "Shift+A");
await page.keyboard.press("CapsLock");
await page.getByText("Caps Lock").and(page.locator(".on")).waitFor();
await page.keyboard.down("Control");
await page.keyboard.down("Alt");
await page.keyboard.up("Alt");
await page.keyboard.up("Control");
await page.getByText("Click to Capture").waitFor();
await expectReport(/report input1 00 00 00 00 00 00 00 00\n(?![\s\S]*report input1 0[^0])/, "the release of every key");

await page.getByRole("button", { name: "Devices" }).click();
await page.getByText("Paired Devices").waitFor();
await shot("devices");
await page.getByRole("button", { name: "Key Combinations" }).click();
await shot("key-combinations");
await page.getByRole("button", { name: "Edit" }).first().click();
await shot("edit-key-combination");
await page.getByRole("button", { name: "Cancel" }).click();
await page.getByRole("button", { name: "Settings" }).click();
await shot("settings");
await page.mouse.wheel(0, 2000);
await shot("settings-diagnostics");

await app.close();
if (errors.length) {
  console.error(errors.join("\n"));
  process.exit(1);
}
console.log(`Screenshots in ${out}`);
