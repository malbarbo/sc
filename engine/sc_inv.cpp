// Builds the clang CompilerInvocation for sc directly, without the cc1 option
// parser (CompilerInvocation::CreateFromArgs and the option tables).
//
// The result must match what `clang -cc1` produces for the flags the engine
// always uses; checkinv.cpp (native) compares both via generateCC1CommandLine.
#include "sc_inv.h"
#include "clang/Basic/LangOptions.h"
#include "clang/Basic/LangStandard.h"
#include "clang/Basic/TargetOptions.h"
#include "clang/Lex/HeaderSearchOptions.h"
#include "clang/Frontend/CompilerInvocation.h"
#include "clang/StaticAnalyzer/Core/AnalyzerOptions.h"
#include "clang/Frontend/FrontendOptions.h"
#include "llvm/IR/DebugInfoMetadata.h"
#include "llvm/TargetParser/Triple.h"

using namespace clang;

void scBuildInvocation(CompilerInvocation &Inv, const ScOptions &O) {
  llvm::Triple T("wasm32-unknown-wasip1");

  TargetOptions &TO = Inv.getTargetOpts();
  TO.Triple = T.str();
  TO.CPU = "generic";
  TO.CodeModel = "default";
  TO.CodeObjectVersion = llvm::CodeObjectVersionKind::COV_6;
  TO.HLSLEntry = "main";
  TO.DxilValidatorVersion = "1.8";

  LangOptions &LO = Inv.getLangOpts();
  std::vector<std::string> Includes;
  Language L = O.CXX ? Language::CXX : Language::C;
  LangOptions::setLangDefaults(LO, L, T, Includes,
                              O.CXX ? LangStandard::lang_gnucxx20 : LangStandard::lang_unspecified);
  LO.GNUCVersion = 40201;
  LO.setDefaultFPContractMode(LangOptions::FPM_On);
  LO.setValueVisibilityMode(HiddenVisibility);
  // defaults of the cc1 option marshalling that differ from the struct defaults
  LO.setTypeVisibilityMode(HiddenVisibility);
  LO.ConvergentFunctions = false;
  LO.BracketDepth = 2048;
  LO.CFRuntime = LangOptions::CoreFoundationABI::ObjectiveC;
  LO.ObjCDefaultSynthProperties = true;
  LO.MaxBitIntWidth = 0;
  LO.MathErrno = false;
  LO.ModulesSearchAll = false;
  LO.EnableLifetimeSafety = true;
  LO.WCharIsSigned = true;
  LO.setComplexRange(LangOptions::CX_Full);
  // marshalled options whose default is another option (invisible to the round trip)
  LO.CheckConstexprFunctionBodies = true;
  LO.Coroutines = false;
  LO.CPlusPlusModules = false;
  LO.DollarIdents = true;
  LO.FastMath = false;
  LO.GNUKeywords = LO.GNUMode;
  LO.GNUInline = !LO.C99 && !LO.CPlusPlus;
  LO.AsmBlocks = false;
  LO.Modules = false;
  LO.ModulesDeclUse = false;
  LO.CXXOperatorNames = false;
  LO.SizedDeallocation = false;
  LO.AlignedAllocation = false;
  LO.RTTI = false;
  LO.RTTIData = false;
  LO.Char8 = false;
  LO.NativeInt16Type = true;
  LO.HLSLRootSigVer = llvm::dxbc::RootSignatureVersion::V1_1;
  if (O.CXX) {
    // gnu++20, -fno-exceptions (wasi-sdk's libc++ is built without them)
    LO.CXXOperatorNames = true;
    LO.RTTI = LO.RTTIData = true;
    LO.SizedDeallocation = LO.CPlusPlus14;
    LO.AlignedAllocation = LO.CPlusPlus17;
    LO.Char8 = LO.CPlusPlus20;
    LO.Coroutines = LO.CPlusPlus20;
    LO.CPlusPlusModules = LO.Modules = LO.ModulesLocalVisibility = LO.CPlusPlus20;
    LO.ImplicitModules = false;
    LO.Deprecated = true;
    LO.SkipODRCheckInGMF = true;
  }

  CodeGenOptions &CG = Inv.getCodeGenOpts();
  CG.DisableFree = true;
  CG.ClearASTBeforeBackend = true;
  CG.VerifyModule = false;
  CG.DiscardValueNames = true;
  CG.RelocationModel = llvm::Reloc::Static;
  CG.setFramePointer(CodeGenOptions::FramePointerKind::None);
  CG.BBSections = "none";
  CG.NoCommon = true;
  CG.setAssignmentTrackingMode(CodeGenOptions::AssignmentTrackingOpts::Enabled);
  CG.MergeAllConstants = false;
  CG.DirectAccessExternalData = true;
  CG.StrictLifetimes = true;
  CG.SanitizeAddressUseOdrIndicator = true;
  CG.SanitizeMemoryParamRetval = true;
  CG.UseInitArray = true;
  CG.AsmVerbose = true;
  CG.RegisterGlobalDtorsWithAtExit = false;
  CG.SplitDwarfInlining = false;
  CG.UniqueBasicBlockSectionNames = false;
  CG.XRayInstrumentationBundle.Mask = XRayInstrKind::All;
  CG.CodeViewCommandLine = true;
  CG.DebugColumnInfo = true;
  CG.DebugStructorDeclLinkageNames = true;
  CG.DebugNameTable = static_cast<unsigned>(llvm::DICompileUnit::DebugNameTableKind::None);
  CG.DebugStrictDwarf = false;
  CG.VerifyEach = false;
  CG.IncrementalLinkerCompatible = true;
  CG.EnableNoundefAttrs = true;
  CG.StructPathTBAA = true;
  CG.SSPBufferSize = 8;
  // set by ParseCodeGenArgs / FixupInvocation outside the marshalling
  CG.OptimizationLevel = 0;
  CG.setInlining(CodeGenOptions::OnlyAlwaysInlining);
  CG.SimplifyLibCalls = !LO.NoBuiltin;
  CG.NewStructPathTBAA = false;
  CG.PrepareForLTO = false;
  CG.PrepareForThinLTO = false;
  CG.EmitVersionIdentMetadata = true;
  CG.EmitIEEENaNCompliantInsts = true;
  CG.CodeModel = TO.CodeModel;
  if (O.CXX)
    CG.CXXCtorDtorAliases = true;

  DiagnosticOptions &DO = Inv.getDiagnosticOpts();
  DO.Warnings = O.Warnings;
  DO.ErrorLimit = 19;
  DO.ShowOptionNames = true;
  DO.ShowPresumedLoc = true;
  DO.ElideType = true;
  DO.ShowColors = false;

  AnalyzerOptions &AO = Inv.getAnalyzerOpts();
  AO.ShouldEmitErrorsOnInvalidConfigValue = true;
  AO.InlineMaxStackDepth = 5;
  AO.maxBlockVisitOnPath = 4;

  HeaderSearchOptions &HS = Inv.getHeaderSearchOpts();
  HS.Sysroot = O.Sysroot;
  HS.ResourceDir = O.ResourceDir;
  for (const auto &P : O.ISystem)
    HS.AddPath(P, frontend::System, false, true);
  for (const auto &P : O.ExternCISystem)
    HS.AddPath(P, frontend::ExternCSystem, false, true);

  FrontendOptions &FO = Inv.getFrontendOpts();
  FO.DisableFree = true;
  FO.ProgramAction = frontend::EmitObj;
  FO.DashX = InputKind(L);
  FO.OutputFile = O.Output;
  FO.Inputs.push_back(FrontendInputFile(O.Input, InputKind(L)));
}

// Accepts the subset of cc1 flags the engine passes; everything else is fixed.
bool scParseArgs(llvm::ArrayRef<const char *> Args, ScOptions &O) {
  for (size_t i = 0; i < Args.size(); ++i) {
    llvm::StringRef A = Args[i];
    auto Next = [&]() -> const char * { return i + 1 < Args.size() ? Args[++i] : ""; };
    if (A == "-o") O.Output = Next();
    else if (A == "-isysroot") O.Sysroot = Next();
    else if (A == "-resource-dir") O.ResourceDir = Next();
    else if (A == "-internal-isystem") O.ISystem.push_back(Next());
    else if (A == "-internal-externc-isystem") O.ExternCISystem.push_back(Next());
    else if (A == "-triple" || A == "-target-cpu" || A == "-mrelocation-model" || A == "-ferror-limit")
      Next();
    else if (A.starts_with("-W"))
      O.Warnings.push_back(A.drop_front(2).str());
    else if (!A.starts_with("-")) {
      O.Input = A.str();
      O.CXX = A.ends_with(".cpp") || A.ends_with(".cc") || A.ends_with(".cxx");
    }
  }
  return !O.Input.empty();
}
