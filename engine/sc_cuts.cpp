// Replacements for LLVM/clang pieces that sc does not need but that are still
// referenced. Defined here, the archive members that hold the originals are
// never pulled into the link.
#include "llvm/Support/Unicode.h"

namespace llvm::sys::unicode {
// Unicode character names (\N{...} escapes): drops the 240 KB name table.
// Every name is unknown, so clang reports an "invalid Unicode character name".
std::optional<char32_t> nameToCodepointStrict(StringRef) { return std::nullopt; }
std::optional<LooseMatchingResult> nameToCodepointLooseMatching(StringRef) { return std::nullopt; }
SmallVector<MatchForCodepointName> nearestMatchesForCodepointName(StringRef, std::size_t) {
  return {};
}
} // namespace llvm::sys::unicode

#ifndef __wasm__
#include <cstdio>
#include <cstdlib>
// The native engine: the stub of a cut function (scripts/link.py --native)
// jumps here with the function's name.
extern "C" [[noreturn]] void sc_cut_called(const char *Name) {
  fprintf(stderr, "sc: cut function called: %s\n", Name);
  abort();
}
#endif
