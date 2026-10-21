// sc compiler core, v2: clang frontend (invocation built in code, see
// sc_inv.cpp), our own O0 backend (sc_backend.cpp) and wasm-ld in one module.
// sc_main is the one-shot engine (sc.h); sc_entry.cpp is the executable's main.
//   sc.wasm -cc1 <cc1 args...>   compile one TU (only the flags sc_inv knows), or
//                                build a module's .pcm with -emit-module-interface
//   sc.wasm ld <wasm-ld args...> link
#include "sc.h"
#include "sc_inv.h"
#include "clang/Basic/DiagnosticFrontend.h"
#include "clang/CodeGen/CodeGenAction.h"
#include "clang/Frontend/CompilerInstance.h"
#include "clang/Frontend/CompilerInvocation.h"
#include "clang/Frontend/FrontendActions.h"
#include "lld/Common/Driver.h"
#include "llvm/Support/BuryPointer.h"
#include "llvm/Support/ErrorHandling.h"
#include "llvm/Support/Process.h"
#include "llvm/Support/Signals.h"
#include "llvm/Support/VirtualFileSystem.h"
#include "llvm/Support/raw_ostream.h"
#include <cstring>

LLD_HAS_DRIVER(wasm)

extern "C" {
void LLVMInitializeWebAssemblyTargetInfo();
void LLVMInitializeWebAssemblyTarget();
void LLVMInitializeWebAssemblyTargetMC();
void LLVMInitializeWebAssemblyAsmPrinter();
void LLVMInitializeWebAssemblyAsmParser();
}

static void errorHandler(void *UserData, const char *Message, bool GenCrashDiag) {
  auto &Diags = *static_cast<clang::DiagnosticsEngine *>(UserData);
  Diags.Report(clang::diag::err_fe_error_backend) << Message;
  llvm::sys::RunInterruptHandlers();
  llvm::sys::Process::Exit(GenCrashDiag ? 70 : 1);
}

static int cc1(llvm::ArrayRef<const char *> Args) {
  ScOptions O;
  std::string Error;
  if (!scParseArgs(Args, O, Error)) {
    llvm::errs() << "sc: " << Error << "\n";
    return 1;
  }
  if (O.Input.empty()) {
    llvm::errs() << "sc: no input file\n";
    return 1;
  }
  LLVMInitializeWebAssemblyTargetInfo();
  LLVMInitializeWebAssemblyTarget();
  LLVMInitializeWebAssemblyTargetMC();
  LLVMInitializeWebAssemblyAsmPrinter();
  LLVMInitializeWebAssemblyAsmParser();

  auto Inv = std::make_shared<clang::CompilerInvocation>();
  scBuildInvocation(*Inv, O);
  scSetLLVMArgs(*Inv);
  auto Clang = std::make_unique<clang::CompilerInstance>(std::move(Inv));
  Clang->createVirtualFileSystem(llvm::vfs::getRealFileSystem());
  Clang->createDiagnostics();
  llvm::install_fatal_error_handler(errorHandler, &Clang->getDiagnostics());
  bool Ok;
  if (O.ModuleInterface) {
    clang::GenerateModuleInterfaceAction Act;
    Ok = Clang->ExecuteAction(Act);
  } else {
    clang::EmitObjAction Act;
    Ok = Clang->ExecuteAction(Act);
  }
  llvm::remove_fatal_error_handler();
  // as cc1_main with -disable-free: flush the diagnostic client, skip teardown
  Clang->getDiagnosticClient().~DiagnosticConsumer();
  llvm::BuryPointer(std::move(Clang));
  return !Ok;
}

int sc_main(int argc, const char **argv) {
  if (argc > 1 && !strcmp(argv[1], "-cc1"))
    return cc1(llvm::ArrayRef(argv + 2, argv + argc));
  if (argc > 1 && !strcmp(argv[1], "ld")) {
    std::vector<const char *> args = {"wasm-ld"};
    args.insert(args.end(), argv + 2, argv + argc);
    lld::Result r = lld::lldMain(args, llvm::outs(), llvm::errs(), {{lld::Wasm, &lld::wasm::link}});
    return r.retCode;
  }
  llvm::errs() << "usage: sc -cc1 ... | sc ld ...\n";
  return 2;
}
