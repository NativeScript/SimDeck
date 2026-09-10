// Helpers for shipping a self-contained Windows binary from the
// `x86_64-pc-windows-gnu` target.
//
// The OpenH264 encoder is C++ compiled by the MinGW toolchain, which links
// `libstdc++-6.dll` dynamically by default. That DLL only exists inside
// MinGW/Git Bash installs, so a plain PowerShell or cmd launch of the binary
// fails with STATUS_DLL_NOT_FOUND (0xC0000135) and a modal error dialog.

/** MinGW runtime DLLs that a shipped SimDeck binary must never import. */
export const MINGW_RUNTIME_DLLS = [
  "libstdc++-6.dll",
  "libgcc_s_seh-1.dll",
  "libgcc_s_dw2-1.dll",
  "libwinpthread-1.dll",
];

const CRT_STATIC_FLAG = "-C target-feature=+crt-static";

export function isWindowsGnuTarget(target) {
  return typeof target === "string" && /-windows-gnu(llvm)?$/.test(target);
}

/** Cargo's per-target RUSTFLAGS variable, e.g. CARGO_TARGET_X86_64_PC_WINDOWS_GNU_RUSTFLAGS. */
export function targetRustflagsEnvName(target) {
  return `CARGO_TARGET_${target.toUpperCase().replace(/-/g, "_")}_RUSTFLAGS`;
}

/**
 * Adds `+crt-static` to existing RUSTFLAGS so rustc links the executable with
 * `-static`, which resolves the MinGW C++ runtime from static archives.
 */
export function windowsGnuRustflags(existing) {
  const current = existing?.trim() ?? "";
  if (current.includes("+crt-static")) {
    return current;
  }
  return current ? `${current} ${CRT_STATIC_FLAG}` : CRT_STATIC_FLAG;
}

/** Returns the lower-cased DLL names in a PE file's import directory. */
export function peImportedDlls(binary) {
  const view = Buffer.isBuffer(binary) ? binary : Buffer.from(binary);
  if (view.length < 0x40 || view.readUInt16LE(0) !== 0x5a4d) {
    throw new Error("Not a PE file: missing MZ header.");
  }
  const peOffset = view.readUInt32LE(0x3c);
  if (view.readUInt32LE(peOffset) !== 0x00004550) {
    throw new Error("Not a PE file: missing PE signature.");
  }
  const coffOffset = peOffset + 4;
  const sectionCount = view.readUInt16LE(coffOffset + 2);
  const optionalHeaderSize = view.readUInt16LE(coffOffset + 16);
  const optionalOffset = coffOffset + 20;
  const magic = view.readUInt16LE(optionalOffset);
  const dataDirectoryOffset =
    magic === 0x20b
      ? optionalOffset + 112
      : magic === 0x10b
        ? optionalOffset + 96
        : null;
  if (dataDirectoryOffset === null) {
    throw new Error(
      `Unknown PE optional header magic 0x${magic.toString(16)}.`,
    );
  }
  const importTableRva = view.readUInt32LE(dataDirectoryOffset + 8);
  if (importTableRva === 0) {
    return [];
  }

  const sections = [];
  const sectionsOffset = optionalOffset + optionalHeaderSize;
  for (let index = 0; index < sectionCount; index += 1) {
    const offset = sectionsOffset + index * 40;
    const virtualSize = view.readUInt32LE(offset + 8);
    const virtualAddress = view.readUInt32LE(offset + 12);
    const rawSize = view.readUInt32LE(offset + 16);
    const rawOffset = view.readUInt32LE(offset + 20);
    sections.push({
      virtualAddress,
      size: Math.max(virtualSize, rawSize),
      rawOffset,
    });
  }
  const rvaToOffset = (rva) => {
    const section = sections.find(
      (candidate) =>
        rva >= candidate.virtualAddress &&
        rva < candidate.virtualAddress + candidate.size,
    );
    if (!section) {
      throw new Error(`PE RVA 0x${rva.toString(16)} is outside every section.`);
    }
    return rva - section.virtualAddress + section.rawOffset;
  };
  const readCString = (offset) => {
    let end = offset;
    while (end < view.length && view[end] !== 0) {
      end += 1;
    }
    return view.toString("latin1", offset, end);
  };

  const names = [];
  let descriptor = rvaToOffset(importTableRva);
  while (descriptor + 20 <= view.length) {
    const nameRva = view.readUInt32LE(descriptor + 12);
    const firstThunk = view.readUInt32LE(descriptor + 16);
    if (nameRva === 0 && firstThunk === 0) {
      break;
    }
    if (nameRva !== 0) {
      names.push(readCString(rvaToOffset(nameRva)).toLowerCase());
    }
    descriptor += 20;
  }
  return names;
}

/** MinGW runtime DLLs imported by a PE binary, empty for a self-contained one. */
export function mingwRuntimeImports(binary) {
  const imported = new Set(peImportedDlls(binary));
  return MINGW_RUNTIME_DLLS.filter((dll) => imported.has(dll));
}
