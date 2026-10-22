// The files of sc_set_files (sc.h) in sc.wasm: the pack (PLANO-REPL.md
// §5.4) is in the module, a squashfs without compression (build/pack/sc.sqfs,
// scripts/pack.py) that squashfuse's library reads, so the session reads it
// as the native one reads its own and the host gives no files
// (PLANO-REPL.md §5.5). A host calls sc_pack after _initialize. What a read
// gives is copied out of the image once and kept, with a NUL after it, as
// sc_files asks; the image itself is only read, and a host that snapshots
// the memory may leave it out (sc_pack_image).
#include "sc.h"
#include <stdlib.h>
#include <string.h>
#include <sys/stat.h>
#include <string>
#include <unordered_map>
extern "C" {
#include "squashfuse.h"
#include "decompress.h"
}

namespace {
alignas(8) const unsigned char Image[] = {
#embed "sc.sqfs"
};

sqfs Fs;
std::string Root;

struct Bytes {
  char *P;
  unsigned long long N;
};
std::unordered_map<std::string, Bytes> Kept;

// the inode at path, a path under Root
bool inode(const char *path, sqfs_inode *I) {
  size_t N = Root.size();
  if (strncmp(path, Root.c_str(), N) || (path[N] && path[N] != '/'))
    return false;
  bool Found = false;
  return !sqfs_inode_get(&Fs, I, sqfs_inode_root(&Fs)) && !sqfs_lookup_path(&Fs, I, path + N, &Found) && Found;
}

// the bytes of a file, or the names of a directory each followed by a NUL;
// with a NUL after them, kept
const Bytes *get(const char *path) {
  auto It = Kept.find(path);
  if (It != Kept.end())
    return &It->second;
  sqfs_inode I;
  if (!inode(path, &I))
    return nullptr;
  std::string Names;
  if (S_ISDIR(I.base.mode)) {
    sqfs_dir D;
    sqfs_dir_entry E;
    char Name[SQUASHFS_NAME_LEN + 1];
    sqfs_err Err = SQFS_OK;
    if (sqfs_dir_open(&Fs, &I, &D, 0))
      return nullptr;
    sqfs_dentry_init(&E, Name);
    while (sqfs_dir_next(&Fs, &D, &E, &Err))
      Names.append(sqfs_dentry_name(&E), sqfs_dentry_name_size(&E)).push_back(0);
    if (Err)
      return nullptr;
  } else if (!S_ISREG(I.base.mode)) {
    return nullptr;
  }
  sqfs_off_t N = S_ISDIR(I.base.mode) ? Names.size() : I.xtra.reg.file_size;
  char *P = static_cast<char *>(malloc(N + 1));
  if (!P)
    return nullptr;
  if (S_ISDIR(I.base.mode)) {
    memcpy(P, Names.data(), N);
  } else if (sqfs_off_t Got = N; N && (sqfs_read_range(&Fs, &I, 0, &Got, P) || Got != N)) {
    free(P);
    return nullptr;
  }
  P[N] = 0;
  return &Kept.emplace(path, Bytes{P, static_cast<unsigned long long>(N)}).first->second;
}

const char *of(const char *path, bool dir, unsigned long long *size) {
  sqfs_inode I;
  const Bytes *B = inode(path, &I) && S_ISDIR(I.base.mode) == dir ? get(path) : nullptr;
  if (B)
    *size = B->N;
  return B ? B->P : nullptr;
}
} // namespace

// squashfuse reads the image through these: from the module's memory, and
// with no decompressor (scripts/pack.py makes none of it compressed)
extern "C" ssize_t sqfs_pread(sqfs_fd_t, void *buf, size_t count, sqfs_off_t off) {
  if (off < 0 || static_cast<size_t>(off) > sizeof Image)
    return -1;
  count = count < sizeof Image - off ? count : sizeof Image - off;
  memcpy(buf, Image + off, count);
  return count;
}

extern "C" sqfs_decompressor sqfs_decompressor_get(sqfs_compression_type) {
  return [](void *, size_t, void *, size_t *) { return SQFS_BADCOMP; };
}

// the pack, as sc_set_files at root (scripts/flags.py, PACK)
extern "C" __attribute__((export_name("sc_pack"))) void sc_pack(const char *root) {
  static sc_files F;
  if (sqfs_init(&Fs, 0, 0))
    abort();
  Root = root;
  F.root = Root.c_str();
  F.status = [](const char *path, unsigned long long *size) {
    sqfs_inode I;
    if (!inode(path, &I))
      return 0;
    if (S_ISREG(I.base.mode))
      *size = I.xtra.reg.file_size;
    return S_ISDIR(I.base.mode) ? 2 : S_ISREG(I.base.mode) ? 1 : 0;
  };
  F.read = [](const char *path, unsigned long long *size) { return of(path, false, size); };
  F.list = [](const char *path, unsigned long long *size) { return of(path, true, size); };
  sc_set_files(&F);
}

// where the image is in the memory, and its size
extern "C" __attribute__((export_name("sc_pack_image"))) const unsigned char *sc_pack_image(unsigned *size) {
  *size = sizeof Image;
  return Image;
}
