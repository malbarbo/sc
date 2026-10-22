#pragma once
#include "sc_inv.h"
#include "llvm/ADT/IntrusiveRefCntPtr.h"
#include "llvm/ADT/StringRef.h"
#include <functional>
#include <memory>
#include <set>
#include <string>
#include <vector>

namespace llvm { class Module; class raw_ostream; }
namespace clang { class Decl; }
namespace llvm::vfs { class FileSystem; }

// The session's file system: the real one, with the files of sc_set_files
// (sc.h) under their root (sc_files.cpp).
llvm::IntrusiveRefCntPtr<llvm::vfs::FileSystem> scFileSystem();

// While a cell is linked (sc_api.cpp): wasm-ld reads its object Obj at Path
// and writes to Out the output of "-o -", in place of the disk.
struct ScLink {
  ScLink(llvm::StringRef Path, llvm::StringRef Obj, std::string &Out);
  ~ScLink();
};

// What a cell left: SC_OK with the object, SC_ERROR (an error; the session
// keeps part of the cell, §3.3), SC_LOST (the session must be recreated
// and replayed) or SC_INCOMPLETE (an error where the cell ends: SC_CELL_WHOLE).
enum ScStatus { ScOk = 0, ScError = 1, ScLost = 2, ScIncomplete = 3 };

// The internal globals of a module that stand for a declaration (a static
// variable or function, a namespace-scope const, an anonymous namespace)
// become external under a name of the session, so a later cell uses the one
// copy. A definition of a name an earlier cell exports (Known: a promoted
// copy re-emitted, or, from a PCH, a variable of an earlier cell) becomes a
// declaration, and the dynamic initializers that wrote only those are
// emptied. A declaration of an internal name a later cell defines takes the
// name of the promoted definition. DeclOf: the declaration a name of the
// module stands for, if any.
void scPromoteInternals(llvm::Module &M, const std::set<std::string> &Known,
                        const std::function<const clang::Decl *(llvm::StringRef)> &DeclOf);

// The names a cell's object defines for the cells after it.
std::vector<std::string> scExports(const llvm::Module &M);

// Candidate A: one clang session, each cell parsed on top of the ones before
// (clang-repl's IncrementalParser, without clangInterpreter).
class ScSession {
public:
  // O: the flags of a one-shot compile, without an input file. Diags: where
  // the diagnostics go (stderr if null); null if the session cannot start
  static std::unique_ptr<ScSession> create(ScOptions O, llvm::raw_ostream *Diags = nullptr);
  ~ScSession();
  // Backend: false parses and generates IR only (the replay). Whole: a cell
  // whose first error is at its end is ScIncomplete
  ScStatus cell(llvm::StringRef Src, bool Backend, std::string &Obj, std::vector<std::string> &Syms,
                bool Whole = false);
  // The type of Src, an expression without its `;`, as a shown value
  // writes it; ScError if Src is not one. Nothing is generated, but as a
  // failed cell, Src leaves part of itself in the session
  ScStatus type(llvm::StringRef Src, std::string &Type);
  // The brackets Src leaves open, one more if it ends inside a comment or
  // with a `\` that joins the next line
  unsigned depth(llvm::StringRef Src);

  struct Impl;
private:
  std::unique_ptr<Impl> I;
};
