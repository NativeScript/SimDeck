import assert from "node:assert/strict";
import { test } from "node:test";

import {
  isWindowsGnuTarget,
  mingwRuntimeImports,
  peImportedDlls,
  targetRustflagsEnvName,
  windowsGnuRustflags,
} from "./windows-runtime.mjs";

/** Builds a minimal PE32+ image whose import directory names `dlls`. */
function buildPeWithImports(dlls) {
  const sectionRva = 0x1000;
  const sectionRawOffset = 0x200;
  const descriptorsSize = (dlls.length + 1) * 20;
  const nameBlobs = dlls.map((dll) => Buffer.from(`${dll}\0`, "latin1"));
  const namesSize = nameBlobs.reduce((total, blob) => total + blob.length, 0);
  const section = Buffer.alloc(descriptorsSize + namesSize);
  let nameRva = sectionRva + descriptorsSize;
  let nameOffset = descriptorsSize;
  dlls.forEach((_, index) => {
    const descriptor = index * 20;
    section.writeUInt32LE(nameRva, descriptor + 12);
    section.writeUInt32LE(sectionRva + 0x800, descriptor + 16);
    nameBlobs[index].copy(section, nameOffset);
    nameRva += nameBlobs[index].length;
    nameOffset += nameBlobs[index].length;
  });

  const peOffset = 0x40;
  const optionalHeaderSize = 240;
  const image = Buffer.alloc(sectionRawOffset + section.length);
  image.writeUInt16LE(0x5a4d, 0);
  image.writeUInt32LE(peOffset, 0x3c);
  image.writeUInt32LE(0x00004550, peOffset);
  const coffOffset = peOffset + 4;
  image.writeUInt16LE(0x8664, coffOffset);
  image.writeUInt16LE(1, coffOffset + 2);
  image.writeUInt16LE(optionalHeaderSize, coffOffset + 16);
  const optionalOffset = coffOffset + 20;
  image.writeUInt16LE(0x20b, optionalOffset);
  image.writeUInt32LE(sectionRva, optionalOffset + 112 + 8);
  image.writeUInt32LE(section.length, optionalOffset + 112 + 12);
  const sectionHeader = optionalOffset + optionalHeaderSize;
  image.write(".idata", sectionHeader, "latin1");
  image.writeUInt32LE(section.length, sectionHeader + 8);
  image.writeUInt32LE(sectionRva, sectionHeader + 12);
  image.writeUInt32LE(section.length, sectionHeader + 16);
  image.writeUInt32LE(sectionRawOffset, sectionHeader + 20);
  section.copy(image, sectionRawOffset);
  return image;
}

test("windows-gnu targets get +crt-static in their target RUSTFLAGS", () => {
  assert.equal(isWindowsGnuTarget("x86_64-pc-windows-gnu"), true);
  assert.equal(isWindowsGnuTarget("x86_64-pc-windows-gnullvm"), true);
  assert.equal(isWindowsGnuTarget("x86_64-pc-windows-msvc"), false);
  assert.equal(isWindowsGnuTarget("x86_64-unknown-linux-gnu"), false);
  assert.equal(isWindowsGnuTarget(undefined), false);
  assert.equal(
    targetRustflagsEnvName("x86_64-pc-windows-gnu"),
    "CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS",
  );
  assert.equal(windowsGnuRustflags(undefined), "-C target-feature=+crt-static");
  assert.equal(
    windowsGnuRustflags("-C opt-level=3 "),
    "-C opt-level=3 -C target-feature=+crt-static",
  );
  assert.equal(
    windowsGnuRustflags("-C target-feature=+crt-static"),
    "-C target-feature=+crt-static",
  );
});

test("PE import directory lists every imported DLL", () => {
  const image = buildPeWithImports([
    "KERNEL32.dll",
    "ws2_32.dll",
    "libstdc++-6.dll",
  ]);
  assert.deepEqual(peImportedDlls(image), [
    "kernel32.dll",
    "ws2_32.dll",
    "libstdc++-6.dll",
  ]);
});

test("MinGW runtime imports are reported and a static binary passes", () => {
  assert.deepEqual(
    mingwRuntimeImports(
      buildPeWithImports([
        "kernel32.dll",
        "libwinpthread-1.dll",
        "libstdc++-6.dll",
      ]),
    ),
    ["libstdc++-6.dll", "libwinpthread-1.dll"],
  );
  assert.deepEqual(
    mingwRuntimeImports(buildPeWithImports(["kernel32.dll", "ws2_32.dll"])),
    [],
  );
});

test("non-PE input is rejected", () => {
  assert.throws(() => peImportedDlls(Buffer.from("#!/bin/sh\n")), /MZ header/);
});
