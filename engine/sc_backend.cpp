// Replacement for clang/lib/CodeGen/BackendUtil.cpp (left out of the link):
// sc only emits wasm objects at O0, so instead of building the new-PM
// optimization pipeline (PassBuilder pulls in every LLVM pass) we run the O0
// IR pipeline's only relevant pass (AlwaysInliner) and the codegen pipeline in
// a legacy PassManager, like BackendUtil's RunCodegenPipeline.
#include "clang/Basic/CodeGenOptions.h"
#include "clang/Basic/DiagnosticFrontend.h"
#include "clang/Basic/TargetOptions.h"
#include "clang/CodeGen/BackendUtil.h"
#include "clang/Frontend/CompilerInstance.h"
#include "llvm/ADT/StringExtras.h"
#include "llvm/Analysis/RuntimeLibcallInfo.h"
#include "llvm/Analysis/TargetLibraryInfo.h"
#include "llvm/Analysis/TargetTransformInfo.h"
#include "llvm/IR/LegacyPassManager.h"
#include "llvm/InitializePasses.h"
#include "llvm/PassRegistry.h"
#include "llvm/IR/Module.h"
#include "llvm/MC/TargetRegistry.h"
#include "llvm/Target/TargetMachine.h"
#include "llvm/Target/TargetOptions.h"
#include "llvm/Transforms/IPO/AlwaysInliner.h"

using namespace clang;
using namespace llvm;

// The subset of BackendUtil's initTargetOptions that is not a constant for sc.
static void initTargetOptions(const CompilerInstance &CI, llvm::TargetOptions &Options) {
  const auto &CG = CI.getCodeGenOpts();
  const auto &LO = CI.getLangOpts();
  Options.ThreadModel = LO.getThreadModel() == LangOptions::ThreadModelKind::Single
                            ? llvm::ThreadModel::Single
                            : llvm::ThreadModel::POSIX;
  Options.AllowFPOpFusion = LO.getDefaultFPContractMode() == LangOptions::FPM_Fast
                                ? llvm::FPOpFusion::Fast
                                : llvm::FPOpFusion::Standard;
  Options.UseInitArray = CG.UseInitArray;
  Options.NoZerosInBSS = CG.NoZeroInitializedInBSS;
  Options.FunctionSections = CG.FunctionSections;
  Options.DataSections = CG.DataSections;
  Options.UniqueSectionNames = CG.UniqueSectionNames;
  Options.DebuggerTuning = CG.getDebuggerTuning();
  Options.EmitAddrsig = CG.Addrsig;
  Options.MCOptions.EmitDwarfUnwind = CG.getEmitDwarfUnwind();
  Options.MCOptions.MCRelaxAll = CG.RelaxAll;
  Options.MCOptions.MCIncrementalLinkerCompatible = CG.IncrementalLinkerCompatible;
  Options.MCOptions.AsmVerbose = CG.AsmVerbose;
  Options.MCOptions.PreserveAsmComments = CG.PreserveAsmComments;
}

void clang::emitBackendOutput(CompilerInstance &CI, CodeGenOptions &CGOpts,
                              StringRef TDesc, llvm::Module *M, BackendAction Action,
                              IntrusiveRefCntPtr<llvm::vfs::FileSystem> VFS,
                              std::unique_ptr<raw_pwrite_stream> OS, BackendConsumer *BC) {
  DiagnosticsEngine &Diags = CI.getDiagnostics();
  if (Action != Backend_EmitObj) {
    Diags.Report(diag::err_fe_unable_to_interface_with_target);
    return;
  }
  const Triple &TT = M->getTargetTriple();
  std::string Error;
  const llvm::Target *T = TargetRegistry::lookupTarget(TT, Error);
  if (!T) {
    Diags.Report(diag::err_fe_unable_to_create_target) << Error;
    return;
  }
  llvm::TargetOptions Options;
  initTargetOptions(CI, Options);
  const auto &TO = CI.getTargetOpts();
  std::unique_ptr<TargetMachine> TM(T->createTargetMachine(
      TT, TO.CPU, llvm::join(TO.Features, ","), Options, CGOpts.RelocationModel,
      std::nullopt, CodeGenOptLevel::None));
  if (!TM) {
    Diags.Report(diag::err_fe_unable_to_create_target) << "";
    return;
  }
  M->setDataLayout(TM->createDataLayout());

  // the immutable passes below (TargetLibraryInfo, TTI, RuntimeLibraryInfo)
  initializeTarget(*PassRegistry::getPassRegistry());
  initializeAlwaysInlinerLegacyPassPass(*PassRegistry::getPassRegistry());
  legacy::PassManager PM;
  PM.add(createTargetTransformInfoWrapperPass(TM->getTargetIRAnalysis()));
  TargetLibraryInfoImpl TLII(TT);
  PM.add(new TargetLibraryInfoWrapperPass(TLII));
  PM.add(new RuntimeLibraryInfoWrapper(TT, Options.ExceptionModel, Options.FloatABIType,
                                       Options.EABIVersion, Options.MCOptions.ABIName,
                                       Options.VecLib));
  PM.add(createAlwaysInlinerLegacyPass(/*InsertLifetime=*/false));
  if (TM->addPassesToEmitFile(PM, *OS, nullptr, CodeGenFileType::ObjectFile,
                              /*DisableVerify=*/!CGOpts.VerifyModule)) {
    Diags.Report(diag::err_fe_unable_to_interface_with_target);
    return;
  }
  PM.run(*M);

  if (M->getDataLayout().getStringRepresentation() != TDesc)
    Diags.Report(diag::err_data_layout_mismatch)
        << M->getDataLayout().getStringRepresentation() << TDesc;
}

// -fembed-bitcode / -fembed-offload-object are not supported by sc.
void clang::EmbedBitcode(llvm::Module *, const CodeGenOptions &, llvm::MemoryBufferRef) {}
void clang::EmbedObject(llvm::Module *, const CodeGenOptions &, llvm::vfs::FileSystem &,
                        DiagnosticsEngine &) {}
