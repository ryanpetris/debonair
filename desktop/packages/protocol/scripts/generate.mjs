// Generates the protobuf messages from proto/debonair.proto with buf and protoc-gen-es.
// `--check` fails when the checked-in output differs from regeneration.
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const proto = join(root, "../../../proto");
const check = process.argv.includes("--check");

const temporary = mkdtempSync(join(tmpdir(), "debonair-protocol-"));
try {
  // buf only reads module paths inside the current directory, so the schema directory is the
  // input and this package's buf.yaml is passed explicitly. Only the serial API is generated; the
  // storage records stay on the Dongle.
  execFileSync("buf", ["generate", proto, "--path", join(proto, "debonair.proto"), "--config", join(root, "buf.yaml"), "--template", join(root, "buf.gen.yaml"), "--output", temporary], {
    cwd: root,
    stdio: "inherit",
  });
  const text = readFileSync(join(temporary, "src/gen/debonair_pb.ts"), "utf8");
  const file = join(root, "src/gen/debonair_pb.ts");
  if (!check) writeFileSync(file, text);
  else if (readFileSync(file, "utf8") !== text) {
    console.error("src/gen/debonair_pb.ts is out of date; run npm run generate");
    process.exitCode = 1;
  }
} finally {
  rmSync(temporary, { recursive: true, force: true });
}
