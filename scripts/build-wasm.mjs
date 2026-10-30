// Builds crates/protocol-wasm with wasm-pack into crates/protocol-wasm/pkg,
// which is never committed (CI builds it).
//
// secp256k1 is C, compiled by clang for wasm32. Its archive must be written
// by llvm-ar: the system `ar` (macOS's in particular) writes an index that
// rust-lld can't read, and linking fails with undefined secp256k1 symbols.
// rust-toolchain.toml installs llvm-ar with the llvm-tools component.

import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join } from "node:path";

const rustc = (...args) => execFileSync("rustc", args, { encoding: "utf8" });

const env = { ...process.env };
if (!env.AR_wasm32_unknown_unknown) {
  const host = /^host: (.+)$/m.exec(rustc("-vV"))?.[1];
  const ar = join(
    rustc("--print", "sysroot").trim(),
    "lib",
    "rustlib",
    host ?? "",
    "bin",
    "llvm-ar",
  );
  if (!host || !existsSync(ar)) {
    console.error(
      `llvm-ar not found at ${ar}. Run "rustup component add llvm-tools", or set AR_wasm32_unknown_unknown.`,
    );
    process.exit(1);
  }
  env.AR_wasm32_unknown_unknown = ar;
}

execFileSync(
  "wasm-pack",
  [
    "build",
    "crates/protocol-wasm",
    "--target",
    "web",
    "--release",
    "--",
    "--locked",
  ],
  { stdio: "inherit", env },
);
