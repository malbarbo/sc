// The session part of the C API (sc.h) over ScSession (sc_session.cpp): the
// diagnostics go to a string, and each cell's object is linked by wasm-ld
// into a shared object in the session's temporary directory (§3.1).
#include "sc.h"
#include "sc_session.h"
#include "lld/Common/Driver.h"
#include "llvm/Support/FileSystem.h"
#include "llvm/Support/MemoryBuffer.h"
#include "llvm/Support/raw_ostream.h"

LLD_HAS_DRIVER(wasm)

struct sc_session {
  std::unique_ptr<ScSession> S;
  std::vector<std::string> LinkArgs; // after "--"
  llvm::SmallString<128> Dir;        // the cells' objects and shared objects
  std::string Diags, So, Syms;       // the result of the last call
  llvm::raw_string_ostream DiagOS{Diags};
  unsigned N = 0;                    // cells linked
  bool Accepted = false, Lost = false;
  bool OneCell = false; // -sc-one-cell: a symbol no library defines is an error

  ~sc_session() {
    S.reset();
    if (!Dir.empty())
      llvm::sys::fs::remove_directories(Dir);
  }
  void clear() {
    Diags.clear();
    So.clear();
    Syms.clear();
  }
  // the object as a shared object in So; false if wasm-ld failed
  bool link(llvm::StringRef Obj) {
    std::string Base = (Dir + "/cell_" + llvm::Twine(++N)).str();
    std::string O = Base + ".o", SO = Base + ".so";
    {
      std::error_code EC;
      llvm::raw_fd_ostream OS(O, EC);
      if (EC) {
        DiagOS << "sc: " << O << ": " << EC.message() << "\n";
        return false;
      }
      OS << Obj;
    }
    std::vector<const char *> Args = {"wasm-ld", "-shared", "--experimental-pic",
                                      OneCell ? "--unresolved-symbols=report-all" : "--unresolved-symbols=import-dynamic",
                                      O.c_str()};
    for (auto &A : LinkArgs)
      Args.push_back(A.c_str());
    Args.push_back("-o");
    Args.push_back(SO.c_str());
    std::string Out; // without the object's temporary path, which says nothing
    llvm::raw_string_ostream OutOS(Out);
    lld::Result R = lld::lldMain(Args, OutOS, OutOS, {{lld::Wasm, &lld::wasm::link}});
    for (size_t P; (P = Out.find(O + ": ")) != std::string::npos;)
      Out.erase(P, O.size() + 2);
    DiagOS << Out;
    auto Buf = llvm::MemoryBuffer::getFile(SO);
    llvm::sys::fs::remove(O);
    llvm::sys::fs::remove(SO);
    if (R.retCode || !Buf)
      return false;
    So = (*Buf)->getBuffer().str();
    return true;
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
  scParseArgs(llvm::ArrayRef(Args.begin(), Sep), O);
  s->OneCell = O.OneCell;
  if (std::error_code EC = llvm::sys::fs::createUniqueDirectory("sc", s->Dir)) {
    s->Dir.clear();
    s->DiagOS << "sc: temporary directory: " << EC.message() << "\n";
    return SC_ERROR;
  }
  s->S = ScSession::create(O, &s->DiagOS);
  return s->S ? SC_OK : SC_ERROR;
}

void sc_session_free(sc_session *s) { delete s; }

int sc_session_cell(sc_session *s, const char *src, unsigned len, int mode) {
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
