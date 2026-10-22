// The files of sc_set_files (sc.h) in sc.wasm, from its host: a host with a
// pack (src/engine/wasm.rs, test/node/engine.mjs, the page) gives it through
// the imports sc_files.status and sc_files.read and calls sc_host_files, so
// the session reads it as the native one does (PLANO-REPL.md §5.5). What the
// host gives is copied into the guest's memory once and kept, as sc_files
// asks. Without the call, nothing calls the imports: `wasmtime run` runs the
// one-shot engine with -W unknown-imports-trap.
#include "sc.h"
#include <stdlib.h>
#include <string.h>
#include <string>
#include <unordered_map>

/* 0: no such path; 1: a file of *size bytes; 2: a directory, whose names,
   each followed by a NUL, are *size bytes */
__attribute__((import_module("sc_files"), import_name("status"))) int
hostStatus(const char *path, unsigned len, unsigned long long *size);
/* the bytes of a file or the names of a directory, *size of status */
__attribute__((import_module("sc_files"), import_name("read"))) void
hostRead(const char *path, unsigned len, char *out);

namespace {
struct Bytes {
  const char *P;
  unsigned long long N;
  int Kind;
};
std::unordered_map<std::string, Bytes> Kept;

// the bytes of a path of the given kind, with a NUL after them; NULL if it
// is not of that kind
const char *get(const char *path, int kind, unsigned long long *size) {
  auto It = Kept.find(path);
  if (It == Kept.end()) {
    unsigned long long N = 0;
    unsigned Len = strlen(path);
    int K = hostStatus(path, Len, &N);
    if (K != kind)
      return nullptr;
    char *P = static_cast<char *>(malloc(N + 1));
    if (!P)
      return nullptr;
    hostRead(path, Len, P);
    P[N] = 0;
    It = Kept.emplace(path, Bytes{P, N, K}).first;
  }
  if (It->second.Kind != kind)
    return nullptr;
  *size = It->second.N;
  return It->second.P;
}
} // namespace

extern "C" __attribute__((export_name("sc_host_files"))) void sc_host_files(const char *root) {
  static sc_files F;
  F.root = strdup(root);
  F.status = [](const char *path, unsigned long long *size) { return hostStatus(path, strlen(path), size); };
  F.read = [](const char *path, unsigned long long *size) { return get(path, 1, size); };
  F.list = [](const char *path, unsigned long long *size) { return get(path, 2, size); };
  sc_set_files(&F);
}
