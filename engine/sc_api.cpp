// The session part of the C API (sc.h) over ScSession (sc_session.cpp): the
// diagnostics go to a string, and each cell's object is linked by wasm-ld
// into a shared object (§3.1), both in memory (ScLink).
#include "sc.h"
#include "sc_session.h"
#include "lld/Common/Driver.h"
#include "llvm/Support/raw_ostream.h"
#include "llvm/ADT/STLExtras.h"
#include <cerrno>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <ctime>

LLD_HAS_DRIVER(wasm)

struct sc_session {
  std::unique_ptr<ScSession> S;
  std::vector<std::string> LinkArgs; // after "--"
  std::string Diags, So, Syms;       // the result of the last call
  llvm::raw_string_ostream DiagOS{Diags};
  unsigned N = 0;                    // cells linked
  bool Accepted = false, Lost = false;
  bool OneCell = false; // -sc-one-cell: a symbol no library defines is an error

  ~sc_session() { S.reset(); } // before DiagOS, which it may write to
  void clear() {
    Diags.clear();
    So.clear();
    Syms.clear();
  }
  // the object as a shared object in So; false if wasm-ld failed
  bool link(llvm::StringRef Obj) {
    std::string O = "cell_" + std::to_string(++N) + ".o";
    ScLink L(O, Obj, So);
    std::vector<const char *> Args = {"wasm-ld", "-shared", "--experimental-pic",
                                      OneCell ? "--unresolved-symbols=report-all" : "--unresolved-symbols=import-dynamic",
                                      O.c_str()};
    for (auto &A : LinkArgs)
      Args.push_back(A.c_str());
    Args.push_back("-o");
    Args.push_back("-");
    std::string Out; // without the object's name, which says nothing
    llvm::raw_string_ostream OutOS(Out);
    lld::Result R = lld::lldMain(Args, OutOS, OutOS, {{lld::Wasm, &lld::wasm::link}});
    for (size_t P; (P = Out.find(O + ": ")) != std::string::npos;)
      Out.erase(P, O.size() + 2);
    DiagOS << Out;
    return !R.retCode && !So.empty();
  }
};

int sc_session_new(int argc, const char *const *argv, sc_session **out) {
  auto *s = new sc_session;
  *out = s;
  llvm::ArrayRef<const char *> Args(argv, argc);
  auto Sep = llvm::find_if(Args, [](const char *A) { return llvm::StringRef(A) == "--"; });
  if (Sep != Args.end())
    for (const char *A : llvm::ArrayRef(Sep + 1, Args.end()))
      s->LinkArgs.push_back(A);
  ScOptions O;
  std::string Error;
  if (!scParseArgs(llvm::ArrayRef(Args.begin(), Sep), O, Error)) {
    s->DiagOS << "sc: " << Error << "\n";
    return SC_ERROR;
  }
  s->OneCell = O.OneCell;
  s->S = ScSession::create(O, &s->DiagOS);
  return s->S ? SC_OK : SC_ERROR;
}

void sc_session_free(sc_session *s) { delete s; }

static int cell(sc_session *s, const char *src, unsigned len, int mode) {
  s->clear();
  if (mode != SC_CELL_SO && mode != SC_CELL_REPLAY && mode != SC_CELL_WHOLE && mode != SC_CELL_TYPE) {
    s->DiagOS << "sc: unknown cell mode\n";
    return SC_ERROR;
  }
  if (s->Lost || !s->S) {
    s->DiagOS << "sc: the session is lost\n";
    return SC_LOST;
  }
  if (mode == SC_CELL_TYPE)
    return s->S->type(llvm::StringRef(src, len), s->Syms);
  std::string Obj;
  std::vector<std::string> Syms;
  ScStatus St = s->S->cell(llvm::StringRef(src, len), mode != SC_CELL_REPLAY, Obj, Syms, mode == SC_CELL_WHOLE);
  if (St == ScIncomplete) {
    s->Diags.clear(); // what the parser says of a cell that stops short
    return SC_INCOMPLETE;
  }
  if (St == ScOk) {
    s->Accepted = true;
    for (auto &N : Syms)
      s->Syms += N + "\n";
    if (mode != SC_CELL_REPLAY && !s->link(Obj))
      St = ScLost; // the code is generated, the session cannot go back
  }
  if (St == ScLost)
    s->Lost = true;
  return St;
}

#ifdef __wasm__
// The cell that fails, without fork (ANALISE.md §65): with SC_SNAPSHOT in
// the environment, the session copies the linear memory before a cell and
// puts it back if the cell fails or is a type, as the fork of serve
// (src/engine/session.rs) and test/node/engine.mjs do. Only the engine runs
// during the call, so what else lives in the memory (the host's code in
// sc.wasm) is put back as it was. Left out: the stack, whose frames of the
// cell are gone when it returns, and the pack's image, which is only read.
//
// The copy is above the heap, at the top of the memory: sbrk here gives
// malloc the memory below it, and moves it up when the heap grows. The heap
// so sees what it sees without the copy (and with the host's snapshot): the
// same addresses, and clang the same order where it iterates over pointers.
// What the heap grew in the cell is zeroed, as it was.
//
// A trap (Ctrl-C, the host's time limit) may stop the cell anywhere: what
// says where the copy is (Brk, Snap, Mv) is not in it, so that putting it
// back does not change that, and sbrk's move of it can be ended by recover.
//
// SC_SNAPSHOT_LOG appends to that file, for each cell, the heap's size and
// the ms to copy and to put back (or -).
extern "C" char __stack_low[], __stack_high[];
extern "C" const unsigned char *sc_pack_image(unsigned *size);

namespace {
size_t memory() { return __builtin_wasm_memory_size(0) * 65536; }

// the heap's end, what memory.size is without the copy; the copy is from
// here to the memory's end
size_t Brk;

// sbrk's move of the copy up by Grow, from From (the Brk before): Moved of
// its bytes, from its top, are there. Each piece of at most Grow goes where
// none of what is still to move is, so the piece a trap stopped can move
// again
struct Move {
  size_t From, Grow, Moved;
} Mv;

// the order of the stores, where a trap may stop the code
void fence() { __atomic_signal_fence(__ATOMIC_SEQ_CST); }

// at least N bytes for the copy, from Brk
bool room(size_t N) {
  size_t M = memory();
  return Brk + N <= M || __builtin_wasm_memory_grow(0, (Brk + N - M + 65535) / 65536) != SIZE_MAX;
}

struct Snapshot {
  size_t Size = 0; // the heap's when taken
  size_t Used = 0; // the copy's bytes, during a cell

  // the pieces of [8, Size) out of the holes, in order
  template <class F> void pieces(F f) const {
    unsigned N;
    const unsigned char *Img = sc_pack_image(&N);
    auto hole = [](const auto &V) { return std::pair((size_t)&V, (size_t)(&V + 1)); };
    std::pair<size_t, size_t> Holes[] = {{(size_t)__stack_low, (size_t)__stack_high},
                                         {(size_t)Img, (size_t)Img + N},
                                         hole(Brk),
                                         hole(*this),
                                         hole(Mv)};
    llvm::sort(Holes);
    size_t At = 8;
    for (auto [A, B] : Holes) {
      if (A > At)
        f(At, std::min(A, Size));
      At = std::max(At, B);
      if (At >= Size)
        return;
    }
    if (At < Size)
      f(At, Size);
  }

  bool take() {
    Size = Brk;
    size_t N = 0;
    pieces([&](size_t A, size_t B) { N += B - A; });
    if (!room(N))
      return false;
    // Used after the copy: without it, a trap loses the session
    size_t At = 0;
    pieces([&](size_t A, size_t B) {
      memcpy((char *)Brk + At, (const char *)A, B - A);
      At += B - A;
    });
    fence();
    Used = At;
    return true;
  }

  // back to the copy, with the session's diagnostics and names kept (or
  // none, after a trap)
  void restore(sc_session *s, bool Keep = true) {
    size_t Top = Brk, U = Used, D = Keep ? s->Diags.size() : 0, Y = Keep ? s->Syms.size() : 0;
    if (!room(U + D + Y))
      D = Y = 0;
    char *K = (char *)Top + U;
    memcpy(K, s->Diags.data(), D);
    memcpy(K + D, s->Syms.data(), Y);
    size_t At = 0;
    pieces([&](size_t A, size_t B) {
      memcpy((char *)A, (const char *)Top + At, B - A);
      At += B - A;
    });
    // the heap as it was, but what it grew stays its: as memory.size
    memset((char *)Size, 0, Top - Size);
    // the memory is as before the cell: these allocate from it, and sbrk
    // moves what they are copied from, which Used now covers
    Used = U + D + Y;
    s->Diags.clear();
    s->Diags.reserve(D);
    s->Diags.append((char *)Brk + U, D);
    s->Syms.clear();
    s->Syms.reserve(Y);
    s->Syms.append((char *)Brk + U + D, Y);
    s->So.clear();
  }
};

Snapshot Snap;

// the rest of sbrk's move (Mv)
void move() {
  while (Mv.Moved < Snap.Used) {
    size_t L = std::min(Mv.Grow, Snap.Used - Mv.Moved), At = Snap.Used - Mv.Moved - L;
    memcpy((char *)Mv.From + Mv.Grow + At, (const char *)Mv.From + At, L);
    fence();
    Mv.Moved += L;
    fence();
  }
}

bool Snapping = getenv("SC_SNAPSHOT");

double now() {
  timespec T;
  clock_gettime(CLOCK_MONOTONIC, &T);
  return T.tv_sec * 1e3 + T.tv_nsec / 1e6;
}
} // namespace

// malloc's (wasi-libc's dlmalloc), in place of libc's: the memory below the
// copy, which moves up, its place zeroed as new memory is
extern "C" void *sbrk(intptr_t N) {
  if (!Brk)
    Brk = memory();
  if (N == 0)
    return (void *)Brk;
  if (N < 0 || N % 65536)
    abort();
  size_t Old = Brk, M = memory(), Copy = M - Brk;
  if (__builtin_wasm_memory_grow(0, N / 65536) == SIZE_MAX) {
    errno = ENOMEM;
    return (void *)-1;
  }
  Mv.From = Old;
  Mv.Moved = 0;
  fence();
  Mv.Grow = N;
  fence();
  move();
  memset((char *)Old, 0, std::min<size_t>(N, Copy));
  fence();
  Brk += N; // the move is done
  fence();
  Mv.Grow = 0;
  return (void *)Old;
}
#endif

int sc_session_cell(sc_session *s, const char *src, unsigned len, int mode) {
#ifdef __wasm__
  if (Snapping && mode != SC_CELL_REPLAY && s->S && !s->Lost) {
    if (!Brk)
      Brk = memory();
    double T0 = now();
    bool Took = Snap.take();
    double T1 = now();
    if (Took) {
      int St = cell(s, src, len, mode);
      double Back = -1;
      if (St != SC_OK || mode == SC_CELL_TYPE) {
        double T2 = now();
        Snap.restore(s);
        Back = now() - T2;
        // a cell that failed leaves nothing: the session goes on
        if (St != SC_INCOMPLETE && St != SC_OK)
          St = SC_ERROR;
      }
      Snap.Used = 0;
      if (const char *Log = getenv("SC_SNAPSHOT_LOG"))
        if (FILE *F = fopen(Log, "a")) {
          if (Back < 0)
            fprintf(F, "%zu %.2f -\n", Snap.Size, T1 - T0);
          else
            fprintf(F, "%zu %.2f %.2f\n", Snap.Size, T1 - T0, Back);
          fclose(F);
        }
      return St;
    }
  }
#endif
  return cell(s, src, len, mode);
}

#ifdef __wasm__
namespace {
__attribute__((noinline)) int recover(sc_session *s) {
  // a move a trap stopped
  if (Mv.Grow) {
    if (Brk == Mv.From) {
      move();
      Brk += Mv.Grow;
    }
    Mv.Grow = 0;
  }
  // the copy outlives the cell; Brk and Used, if the cell did not break
  // them, say where it is
  if (!Snap.Used || Brk + Snap.Used > memory())
    return SC_LOST;
  // what the cell said is lost with it
  Snap.restore(s, false);
  Snap.Used = 0;
  return SC_ERROR;
}
} // namespace

int sc_session_restore(sc_session *s) { return recover(s); }

void sc_session_snapshot(int on) { Snapping = on; }

int sc_session_recover(sc_session *s) {
  // the stack of the call that stopped is gone: from its top again, before
  // this function's frame (it has none)
  __asm__ volatile(".globaltype __stack_pointer, i32\n\tlocal.get %0\n\tglobal.set __stack_pointer" : : "r"(__stack_high));
  return recover(s);
}
#endif

int sc_session_depth(sc_session *s, const char *src, unsigned len) {
  return s->S ? s->S->depth(llvm::StringRef(src, len)) : 0;
}

int sc_session_undo(sc_session *s) {
  s->clear();
  if (!s->Accepted) {
    s->DiagOS << "sc: no cell to undo\n";
    return SC_ERROR;
  }
  s->Lost = true;
  return SC_LOST;
}

static const std::string *part(sc_session *s, int p) {
  switch (p) {
  case SC_DIAGS: return &s->Diags;
  case SC_SO: return &s->So;
  case SC_SYMS: return &s->Syms;
  }
  return nullptr;
}

const unsigned char *sc_result_ptr(sc_session *s, int p) {
  const std::string *R = part(s, p);
  return R ? reinterpret_cast<const unsigned char *>(R->data()) : nullptr;
}

unsigned sc_result_len(sc_session *s, int p) {
  const std::string *R = part(s, p);
  return R ? R->size() : 0;
}
