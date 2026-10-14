// Phase 0 of PLANO-REPL.md: the two candidates for compiling REPL cells, for
// test/spike.py. Only in build/sc-spike (the uncut native engine).
//
//   sc-spike spike-a <cc1 flags> [-x c++]
//     A, a session: reads commands on stdin, one per line, and answers each
//     on stdout with "ok <exported names...>", "error" or "lost":
//       cell <src> <obj>   compile the cell and write its object
//       replay <src>       the same, without the backend (the replay, §3.6)
//     The diagnostics go to stderr, after a line "== <src>".
//
//   sc-spike spike-b <cc1 flags> [-include-pch <prev.pch>] -emit-pch <new.pch>
//                    -known <file> -o <obj> <src>
//     B, stateless: the cell becomes a PCH chained on the one before, then
//     the object comes from the PCH, with -fpch-codegen (modules codegen).
//     <file> lists what the earlier cells export; the names this cell exports
//     are printed on stdout.
#include "sc_session.h"
#include "clang/Basic/TargetInfo.h"
#include "clang/CodeGen/CodeGenAction.h"
#include "clang/CodeGen/ModuleBuilder.h"
#include "clang/Frontend/CompilerInstance.h"
#include "clang/Frontend/CompilerInvocation.h"
#include "clang/Frontend/FrontendActions.h"
#include "clang/Lex/PreprocessorOptions.h"
#include "llvm/IR/Module.h"
#include "llvm/Support/MemoryBuffer.h"
#include "llvm/Support/VirtualFileSystem.h"
#include "llvm/Support/raw_ostream.h"
#include <fstream>
#include <functional>
#include <iostream>

using namespace clang;

extern "C" {
void LLVMInitializeWebAssemblyTargetInfo();
void LLVMInitializeWebAssemblyTarget();
void LLVMInitializeWebAssemblyTargetMC();
void LLVMInitializeWebAssemblyAsmPrinter();
void LLVMInitializeWebAssemblyAsmParser();
}

// sc_backend.cpp calls it, if set, before the backend (SC_SPIKE only)
extern std::function<void(llvm::Module &)> scBeforeBackend;

static int spikeA(llvm::ArrayRef<const char *> Args) {
  ScOptions O;
  scParseArgs(Args, O);
  auto S = ScSession::create(O);
  if (!S) {
    std::cout << "lost" << std::endl;
    return 1;
  }
  std::string Line;
  while (std::getline(std::cin, Line)) {
    std::string Cmd = Line.substr(0, Line.find(' '));
    if (Cmd == "quit")
      break;
    llvm::StringRef Rest = llvm::StringRef(Line).drop_front(Cmd.size()).trim();
    auto [Src, Obj] = Rest.split(' ');
    llvm::errs() << "== " << Src << "\n";
    llvm::errs().flush();
    auto Buf = llvm::MemoryBuffer::getFile(Src);
    if (!Buf) {
      std::cout << "error" << std::endl;
      continue;
    }
    std::string Out;
    std::vector<std::string> Syms;
    ScStatus St = S->cell((*Buf)->getBuffer(), Cmd == "cell", Out, Syms);
    llvm::errs().flush();
    if (St == ScOk && Cmd == "cell") {
      std::error_code EC;
      llvm::raw_fd_ostream OS(Obj, EC);
      OS << Out;
    }
    if (St == ScOk) {
      std::cout << "ok";
      for (auto &N : Syms)
        std::cout << ' ' << N;
      std::cout << std::endl;
    } else
      std::cout << (St == ScError ? "error" : "lost") << std::endl;
  }
  return 0;
}

static std::unique_ptr<CompilerInstance> instance(std::shared_ptr<CompilerInvocation> Inv) {
  auto CI = std::make_unique<CompilerInstance>(std::move(Inv));
  CI->createVirtualFileSystem(llvm::vfs::getRealFileSystem());
  CI->createDiagnostics();
  return CI;
}

static int spikeB(llvm::ArrayRef<const char *> Args) {
  ScOptions O;
  std::string PrevPCH, NewPCH, KnownFile;
  std::vector<const char *> Rest;
  for (size_t i = 0; i < Args.size(); ++i) {
    llvm::StringRef A = Args[i];
    if (A == "-include-pch") PrevPCH = Args[++i];
    else if (A == "-emit-pch") NewPCH = Args[++i];
    else if (A == "-known") KnownFile = Args[++i];
    else Rest.push_back(Args[i]);
  }
  if (!scParseArgs(Rest, O))
    return 2;
  O.PIC = O.VisibilityDefault = true;
  LLVMInitializeWebAssemblyTargetInfo();
  LLVMInitializeWebAssemblyTarget();
  LLVMInitializeWebAssemblyTargetMC();
  LLVMInitializeWebAssemblyAsmPrinter();
  LLVMInitializeWebAssemblyAsmParser();

  // 1. the cell as a PCH on top of the one before
  auto Inv = std::make_shared<CompilerInvocation>();
  scBuildInvocation(*Inv, O);
  scSetLLVMArgs(*Inv);
  Inv->getLangOpts().ModulesCodegen = true; // -fpch-codegen
  Inv->getPreprocessorOpts().ImplicitPCHInclude = PrevPCH;
  FrontendOptions &FO = Inv->getFrontendOpts();
  FO.ProgramAction = frontend::GeneratePCH;
  FO.OutputFile = NewPCH;
  FO.Inputs.clear();
  FO.Inputs.push_back(FrontendInputFile(O.Input, InputKind(O.CXX ? Language::CXX : Language::C).getHeader()));
  {
    auto CI = instance(Inv);
    GeneratePCHAction Act;
    if (!CI->ExecuteAction(Act))
      return 1;
  }

  // 2. the object, from the PCH
  std::set<std::string> Known;
  {
    std::ifstream In(KnownFile);
    for (std::string L; std::getline(In, L);)
      Known.insert(L);
  }
  auto Inv2 = std::make_shared<CompilerInvocation>();
  scBuildInvocation(*Inv2, O);
  Inv2->getLangOpts().ModulesCodegen = true;
  Inv2->getCodeGenOpts().ClearASTBeforeBackend = false;
  FrontendOptions &FO2 = Inv2->getFrontendOpts();
  FO2.Inputs.clear();
  FO2.Inputs.push_back(FrontendInputFile(NewPCH, InputKind(O.CXX ? Language::CXX : Language::C,
                                                           InputKind::Precompiled)));
  auto CI = instance(Inv2);
  EmitObjAction Act;
  std::vector<std::string> Syms;
  scBeforeBackend = [&](llvm::Module &M) {
    CodeGenerator *CG = Act.getCodeGenerator();
    // modules codegen emits a header's inline functions only in the object of
    // the PCH that has the header, and libc++ hides most of them
    for (llvm::GlobalValue &GV : M.global_values())
      if (GV.hasHiddenVisibility() && !GV.hasLocalLinkage()) {
        GV.setVisibility(llvm::GlobalValue::DefaultVisibility);
        GV.setDSOLocal(false);
      }
    scPromoteInternals(M, Known, [&](llvm::StringRef Name) { return CG->GetDeclForMangledName(Name); });
    Syms = scExports(M);
  };
  bool Ok = CI->ExecuteAction(Act);
  scBeforeBackend = nullptr;
  if (!Ok)
    return 1;
  for (auto &N : Syms)
    std::cout << N << '\n';
  return 0;
}

int scSpike(int argc, const char **argv) {
  llvm::ArrayRef<const char *> Args(argv + 1, argv + argc);
  if (!strcmp(argv[0], "spike-a"))
    return spikeA(Args);
  if (!strcmp(argv[0], "spike-b"))
    return spikeB(Args);
  return 2;
}
