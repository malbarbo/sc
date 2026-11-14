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
#include "llvm/Analysis/CFG.h"
#include "llvm/Analysis/CGSCCPassManager.h"
#include "llvm/Analysis/LazyCallGraph.h"
#include "llvm/Analysis/OptimizationRemarkEmitter.h"
#include "llvm/Analysis/RuntimeLibcallInfo.h"
#include "llvm/Analysis/TargetLibraryInfo.h"
#include "llvm/Analysis/TargetTransformInfo.h"
#include "llvm/IR/DebugInfoMetadata.h"
#include "llvm/IR/Dominators.h"
#include "llvm/IR/EHPersonalities.h"
#include "llvm/IR/IRBuilder.h"
#include "llvm/IR/InstIterator.h"
#include "llvm/IR/IntrinsicInst.h"
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
#include "llvm/Transforms/Instrumentation/AddressSanitizer.h"
#include "llvm/Transforms/Utils/BasicBlockUtils.h"

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

// A Stop of the host (env.check_interrupt) has to stop the student's code,
// which never asks for it. Each loop, and each function with a call (it may
// recurse), takes a tick at its head, and one tick in TickPeriod calls
// __sc_interrupt, the loader's, which traps if the user asked to stop. It is
// weak: a program run without the loader (the corpus's) has none.
static constexpr int TickPeriod = 1 << 16;

static bool hasCall(Function &F) {
  for (Instruction &I : instructions(F))
    if (isa<CallBase>(I) && !isa<IntrinsicInst>(I))
      return true;
  return false;
}

static void insertInterruptChecks(llvm::Module &M) {
  LLVMContext &Ctx = M.getContext();
  llvm::Type *I32 = llvm::Type::getInt32Ty(Ctx);
  GlobalVariable *Ticks = nullptr;
  Function *Interrupt = nullptr;
  for (Function &F : M) {
    if (F.isDeclaration() || F.hasAvailableExternallyLinkage() || F.hasFnAttribute(Attribute::Naked))
      continue;
    DISubprogram *SP = F.getSubprogram();
    DebugLoc Head = SP ? DebugLoc(DILocation::get(Ctx, SP->getScopeLine(), 0, SP)) : DebugLoc();
    // where a tick goes, before any of them splits a block
    SmallVector<std::pair<Instruction *, DebugLoc>> At;
    if (hasCall(F))
      At.push_back({&*F.getEntryBlock().getFirstNonPHIOrDbgOrAlloca(), Head});
    SmallVector<std::pair<const BasicBlock *, const BasicBlock *>> Edges;
    FindFunctionBackedges(F, Edges);
    SmallPtrSet<Instruction *, 8> Seen;
    for (auto [From, To] : Edges) {
      auto *Src = const_cast<BasicBlock *>(From);
      auto *Dst = const_cast<BasicBlock *>(To);
      DebugLoc DL = Src->getTerminator()->getDebugLoc();
      if (!DL)
        DL = Head;
      // a catchswitch has no room for a tick: the edge's source takes it
      Instruction *I = Dst->getFirstInsertionPt() != Dst->end() ? &*Dst->getFirstInsertionPt()
                       : Src->getFirstInsertionPt() != Src->end() ? Src->getTerminator()
                                                                   : nullptr;
      if (I && Seen.insert(I).second)
        At.push_back({I, DL});
    }
    if (At.empty())
      continue;
    if (!Ticks) {
      Ticks = new GlobalVariable(M, I32, false, GlobalValue::InternalLinkage, ConstantInt::get(I32, TickPeriod),
                                 "__sc_ticks");
      GlobalValue::SanitizerMetadata Meta;
      Meta.NoAddress = true; // ASan's redzone and registration are for the student's globals
      Ticks->setSanitizerMetadata(Meta);
      Interrupt = Function::Create(llvm::FunctionType::get(llvm::Type::getVoidTy(Ctx), false), GlobalValue::ExternalWeakLinkage,
                                   "__sc_interrupt", M);
    }
    SmallVector<CallInst *> Calls;
    for (auto [I, DL] : At) {
      IRBuilder<> B(I);
      B.SetCurrentDebugLocation(DL);
      Value *N = B.CreateSub(B.CreateLoad(I32, Ticks), B.getInt32(1));
      B.CreateStore(N, Ticks);
      Instruction *Then = SplitBlockAndInsertIfThen(B.CreateICmpSLT(N, B.getInt32(0)), I->getIterator(), false);
      B.SetInsertPoint(Then);
      B.CreateStore(B.getInt32(TickPeriod), Ticks);
      B.SetInsertPoint(SplitBlockAndInsertIfThen(B.CreateIsNotNull(Interrupt), Then->getIterator(), false));
      Calls.push_back(B.CreateCall(Interrupt->getFunctionType(), Interrupt));
    }
    // WinEHPrepare, which wasm's EH runs, makes a call in a catch or a
    // cleanup without the bundle of its pad unreachable
    if (!F.hasPersonalityFn() || !isScopedEHPersonality(classifyEHPersonality(F.getPersonalityFn())))
      continue;
    DenseMap<BasicBlock *, ColorVector> Colors = colorEHFunclets(F);
    for (CallInst *C : Calls) {
      ColorVector &Cs = Colors[C->getParent()];
      if (Cs.size() != 1) { // unreachable, or in more than one funclet: no call
        C->eraseFromParent();
        continue;
      }
      BasicBlock::iterator Pad = Cs.front()->getFirstNonPHIIt();
      if (Pad == Cs.front()->end() || !Pad->isEHPad())
        continue;
      CallBase *New = CallBase::addOperandBundle(C, LLVMContext::OB_funclet, OperandBundleDef("funclet", &*Pad),
                                                 C->getIterator());
      New->copyMetadata(*C);
      C->eraseFromParent();
    }
  }
}

// The end of the O0 pipeline: PassBuilder's buildCoroWrapper, which lowers
// the llvm.coro.* intrinsics of a C++20 coroutine (the backend cannot select
// them), then the interrupt checks, which would split a coroutine before
// CoroSplit, then ASan's pass, which BackendUtil's addSanitizers adds last
// (docs/plan-checks.md §5.2). The analysis managers hold only what these
// passes ask for: ASan's stack safety analysis is off (-asan-use-stack-safety=0).
static void runModulePasses(llvm::Module &M, TargetMachine &TM, const TargetLibraryInfoImpl &TLII,
                            const CodeGenOptions &CG, const LangOptions &LO) {
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
  insertInterruptChecks(M);
  MPM = ModulePassManager();
  if (LO.Sanitize.has(SanitizerKind::Address)) {
    AddressSanitizerOptions Opts;
    Opts.Recover = CG.SanitizeRecover.has(SanitizerKind::Address);
    Opts.UseAfterScope = CG.SanitizeAddressUseAfterScope;
    Opts.UseAfterReturn = CG.getSanitizeAddressUseAfterReturn();
    // no globals GC on wasm (BackendUtil's asanUseGlobalsGC)
    MPM.addPass(AddressSanitizerPass(Opts, /*UseGlobalGC=*/false, CG.SanitizeAddressUseOdrIndicator,
                                     CG.getSanitizeAddressDtor()));
  }
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
  // the coroutine passes and ASan run after the inliner and before codegen
  {
    legacy::PassManager IPM;
    addImmutable(IPM);
    IPM.add(createAlwaysInlinerLegacyPass(/*InsertLifetime=*/false));
    IPM.run(*M);
  }
  runModulePasses(*M, *TM, TLII, CGOpts, CI.getLangOpts());
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
