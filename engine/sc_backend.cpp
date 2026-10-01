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
#include "llvm/Analysis/AssumptionCache.h"
#include "llvm/Analysis/CGSCCPassManager.h"
#include "llvm/Analysis/LazyCallGraph.h"
#include "llvm/Analysis/OptimizationRemarkEmitter.h"
#include "llvm/Analysis/RuntimeLibcallInfo.h"
#include "llvm/Analysis/TargetLibraryInfo.h"
#include "llvm/Analysis/TargetTransformInfo.h"
#include "llvm/IR/Dominators.h"
#include "llvm/IR/LegacyPassManager.h"
#include "llvm/InitializePasses.h"
#include "llvm/PassRegistry.h"
#include "llvm/IR/Module.h"
#include "llvm/IR/PassInstrumentation.h"
#include "llvm/MC/TargetRegistry.h"
#include "llvm/Target/TargetMachine.h"
#include "llvm/Target/TargetOptions.h"
#include "llvm/Transforms/Coroutines/CoroCleanup.h"
#include "llvm/Transforms/Coroutines/CoroConditionalWrapper.h"
#include "llvm/Transforms/Coroutines/CoroEarly.h"
#include "llvm/Transforms/Coroutines/CoroSplit.h"
#include "llvm/Transforms/IPO/AlwaysInliner.h"
#include "llvm/Transforms/IPO/GlobalDCE.h"

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
  Options.ExceptionModel = llvm::ExceptionHandling::Wasm; // always -fwasm-exceptions
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

// The end of the O0 pipeline (PassBuilder's buildCoroWrapper): lowers the
// llvm.coro.* intrinsics of a C++20 coroutine, which the backend cannot select.
// The analysis managers hold only what these passes ask for.
static void runCoroPasses(llvm::Module &M, TargetMachine &TM, const TargetLibraryInfoImpl &TLII) {
  FunctionAnalysisManager FAM;
  CGSCCAnalysisManager CGAM;
  ModuleAnalysisManager MAM;
  FAM.registerPass([] { return PassInstrumentationAnalysis(); });
  FAM.registerPass([&] { return TM.getTargetIRAnalysis(); });
  FAM.registerPass([&] { return TargetLibraryAnalysis(TLII); });
  FAM.registerPass([] { return OptimizationRemarkEmitterAnalysis(); });
  FAM.registerPass([] { return AssumptionAnalysis(); });
  FAM.registerPass([] { return DominatorTreeAnalysis(); });
  FAM.registerPass([&] { return CGSCCAnalysisManagerFunctionProxy(CGAM); });
  FAM.registerPass([&] { return ModuleAnalysisManagerFunctionProxy(MAM); });
  CGAM.registerPass([] { return PassInstrumentationAnalysis(); });
  CGAM.registerPass([] { return FunctionAnalysisManagerCGSCCProxy(); });
  CGAM.registerPass([&] { return ModuleAnalysisManagerCGSCCProxy(MAM); });
  MAM.registerPass([] { return PassInstrumentationAnalysis(); });
  MAM.registerPass([] { return LazyCallGraphAnalysis(); });
  MAM.registerPass([&] { return FunctionAnalysisManagerModuleProxy(FAM); });
  MAM.registerPass([&] { return CGSCCAnalysisManagerModuleProxy(CGAM); });

  ModulePassManager CoroPM;
  CoroPM.addPass(CoroEarlyPass());
  CGSCCPassManager CGPM;
  CGPM.addPass(CoroSplitPass());
  CoroPM.addPass(createModuleToPostOrderCGSCCPassAdaptor(std::move(CGPM)));
  CoroPM.addPass(CoroCleanupPass());
  CoroPM.addPass(GlobalDCEPass());
  ModulePassManager MPM;
  MPM.addPass(CoroConditionalWrapper(std::move(CoroPM)));
  MPM.run(M, MAM);
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
  TargetLibraryInfoImpl TLII(TT);
  auto addImmutable = [&](legacy::PassManager &PM) {
    PM.add(createTargetTransformInfoWrapperPass(TM->getTargetIRAnalysis()));
    PM.add(new TargetLibraryInfoWrapperPass(TLII));
    PM.add(new RuntimeLibraryInfoWrapper(TT, Options.ExceptionModel, Options.FloatABIType,
                                         Options.EABIVersion, Options.MCOptions.ABIName,
                                         Options.VecLib));
  };
  // the coroutine passes run after the inliner and before codegen
  {
    legacy::PassManager IPM;
    addImmutable(IPM);
    IPM.add(createAlwaysInlinerLegacyPass(/*InsertLifetime=*/false));
    IPM.run(*M);
  }
  runCoroPasses(*M, *TM, TLII);
  legacy::PassManager PM;
  addImmutable(PM);
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
