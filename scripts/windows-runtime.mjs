// Helpers for shipping a self-contained Windows binary.
//
// The OpenH264 encoder is C++. Built with the MinGW toolchain
// (`x86_64-pc-windows-gnu`) it links `libstdc++-6.dll`, which only exists
// inside MinGW/Git Bash installs; built with MSVC without a static CRT it
// links `vcruntime140.dll`/`msvcp140.dll`, which not every machine has. Either
// way a plain PowerShell or cmd launch fails with STATUS_DLL_NOT_FOUND
// (0xC0000135) and a modal error dialog instead of the service URL. The
// release therefore targets `x86_64-pc-windows-msvc` with `+crt-static`, and
// the packaging step refuses any binary that still imports a runtime DLL.

/** Toolchain runtime DLLs that a shipped SimDeck binary must never import. */
export const WINDOWS_RUNTIME_DLLS = [
  // MinGW
  "libstdc++-6.dll",
  "libgcc_s_seh-1.dll",
  "libgcc_s_dw2-1.dll",
  "libwinpthread-1.dll",
  // MSVC redistributable
  "vcruntime140.dll",
  "vcruntime140_1.dll",
  "msvcp140.dll",
  "msvcp140_1.dll",
  "msvcp140_2.dll",
  "concrt140.dll",
];

const CRT_STATIC_FLAG = "-C target-feature=+crt-static";

export function isWindowsTarget(target) {
  return typeof target === "string" && /-windows-/.test(target);
}

/** Cargo's per-target RUSTFLAGS variable, e.g. CARGO_TARGET_X86_64_PC_WINDOWS_MSVC_RUSTFLAGS. */
export function targetRustflagsEnvName(target) {
  return `CARGO_TARGET_${target.toUpperCase().replace(/-/g, "_")}_RUSTFLAGS`;
}

/**
 * Adds `+crt-static` to existing RUSTFLAGS so rustc and the `cc` crate link
 * the C/C++ runtime statically (`/MT` on MSVC).
 */
export function windowsRustflags(existing) {
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

/** Toolchain runtime DLLs imported by a PE binary, empty for a self-contained one. */
export function windowsRuntimeImports(binary) {
  const imported = new Set(peImportedDlls(binary));
  return WINDOWS_RUNTIME_DLLS.filter((dll) => imported.has(dll));
}
