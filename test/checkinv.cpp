// Native check: the invocation built by scBuildInvocation must round-trip to
// the same cc1 command line as the one clang's option parser builds.
//   checkinv <cc1 args...>
#include "sc_inv.h"
#include "clang/Basic/DiagnosticIDs.h"
#include "clang/Basic/DiagnosticOptions.h"
#include "clang/Frontend/CompilerInvocation.h"
#include "clang/Frontend/TextDiagnosticPrinter.h"
#include "llvm/Support/raw_ostream.h"
#include "clang/Basic/TargetInfo.h"
#include "clang/Frontend/CompilerInstance.h"
#include "clang/Lex/Preprocessor.h"
#include "llvm/Support/VirtualFileSystem.h"
#include <set>
extern "C" void LLVMInitializeWebAssemblyTargetInfo();

using namespace clang;

static std::vector<std::string> gen(const CompilerInvocation &I) {
  std::vector<std::string> V;
  I.generateCC1CommandLine([&](const llvm::Twine &A) { V.push_back(A.str()); });
  return V;
}

int main(int argc, const char **argv) {
  llvm::ArrayRef<const char *> Args(argv + 1, argv + argc);
  DiagnosticOptions DO;
  DiagnosticsEngine Diags(DiagnosticIDs::create(), DO, new TextDiagnosticPrinter(llvm::errs(), DO));
  CompilerInvocation Parsed;
  if (!CompilerInvocation::CreateFromArgs(Parsed, Args, Diags, "clang"))
    return 1;
  ScOptions O;
  scParseArgs(Args, O);
  CompilerInvocation Mine;
  scBuildInvocation(Mine, O);
  auto A = gen(Parsed), B = gen(Mine);
  std::multiset<std::string> SA(A.begin(), A.end()), SB(B.begin(), B.end());
  int diffs = 0;
  // compare as sequences of (flag [value]) tokens: print tokens missing on either side
  for (auto &x : SA) if (!SB.count(x)) { llvm::outs() << "parser only: " << x << "\n"; ++diffs; }
  for (auto &x : SB) if (!SA.count(x)) { llvm::outs() << "sc only:     " << x << "\n"; ++diffs; }
  // language options field by field: some are never written on the command line
  const LangOptions &LA = Parsed.getLangOpts(), &LB = Mine.getLangOpts();
#define LANGOPT(Name, Bits, Default, Compatibility, Description)                  \
  if (LA.Name != LB.Name) {                                                       \
    llvm::outs() << "langopt " #Name ": parser " << (unsigned)LA.Name << ", sc "   \
                 << (unsigned)LB.Name << "\n";                                     \
    ++diffs;                                                                      \
  }
#define ENUM_LANGOPT(Name, Type, Bits, Default, Compatibility, Description)       \
  if (LA.get##Name() != LB.get##Name()) {                                         \
    llvm::outs() << "langopt " #Name ": parser " << (unsigned)LA.get##Name()      \
                 << ", sc " << (unsigned)LB.get##Name() << "\n";                   \
    ++diffs;                                                                      \
  }
#include "clang/Basic/LangOptions.def"
  // predefined macros (not covered by the command line round trip)
  auto predefs = [&](CompilerInvocation &I) {
    auto CI = std::make_unique<CompilerInstance>(std::make_shared<CompilerInvocation>(I));
    CI->createVirtualFileSystem(llvm::vfs::getRealFileSystem());
    CI->createDiagnostics();
    CI->setTarget(TargetInfo::CreateTargetInfo(CI->getDiagnostics(), CI->getInvocation().getTargetOpts()));
    CI->createFileManager();
    CI->createSourceManager();
    CI->createPreprocessor(TU_Complete);
    return CI->getPreprocessor().getPredefines();
  };
  LLVMInitializeWebAssemblyTargetInfo();
  std::string PA = predefs(Parsed), PB = predefs(Mine);
  if (PA != PB) {
    ++diffs;
    llvm::SmallVector<llvm::StringRef> LA, LB;
    llvm::StringRef(PA).split(LA, '\n'); llvm::StringRef(PB).split(LB, '\n');
    std::set<llvm::StringRef> SLA(LA.begin(), LA.end()), SLB(LB.begin(), LB.end());
    for (auto &x : LA) if (!SLB.count(x)) llvm::outs() << "predef parser only: " << x << "\n";
    for (auto &x : LB) if (!SLA.count(x)) llvm::outs() << "predef sc only:     " << x << "\n";
  }
  llvm::outs() << (diffs ? "DIFFERENT\n" : "SAME\n");
  llvm::outs() << "parser: "; for (auto &x : A) llvm::outs() << x << " "; llvm::outs() << "\n";
  return diffs != 0;
}
