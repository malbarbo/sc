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
#include "llvm/Support/CommandLine.h"
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
  // -fwasm-exceptions: C++ exceptions and setjmp/longjmp, in the standard form
  // (exnref); the pin still defaults to the legacy one
  for (const char *F : {"+exception-handling", "+multivalue", "+reference-types"})
    TO.FeaturesAsWritten.push_back(F);
  if (O.PIC)
    TO.FeaturesAsWritten.push_back("+mutable-globals");

  LangOptions &LO = Inv.getLangOpts();
  std::vector<std::string> Includes;
  Language L = O.CXX ? Language::CXX : Language::C;
  LangStandard::Kind Std = O.CXX ? LangStandard::lang_gnucxx26 : LangStandard::lang_gnu23;
  if (!O.Std.empty())
    Std = LangStandard::getLangKind(O.Std);
  LangOptions::setLangDefaults(LO, L, T, Includes, Std);
  LO.GNUCVersion = 40201;
  LO.setDefaultFPContractMode(LangOptions::FPM_On);
  Visibility Vis = O.VisibilityDefault ? DefaultVisibility : HiddenVisibility;
  LO.setValueVisibilityMode(Vis);
  // defaults of the cc1 option marshalling that differ from the struct defaults
  LO.setTypeVisibilityMode(Vis);
  if (O.PIC)
    LO.PICLevel = 2;
  LO.ConvergentFunctions = false;
  LO.BracketDepth = 2048;
  LO.CFRuntime = LangOptions::CoreFoundationABI::ObjectiveC;
  LO.ObjCDefaultSynthProperties = true;
  LO.MaxBitIntWidth = 0;
  LO.MathErrno = false;
  LO.ModulesSearchAll = false;
  LO.WCharIsSigned = true;
  LO.setComplexRange(LangOptions::CX_Full);
  // marshalled options whose default is another option (invisible to the round trip)
  LO.CheckConstexprFunctionBodies = !LO.CPlusPlus23;
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
    LO.CXXOperatorNames = true;
    LO.Exceptions = LO.CXXExceptions = true;
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
  CG.RelocationModel = O.PIC ? llvm::Reloc::PIC_ : llvm::Reloc::Static;
  CG.setFramePointer(CodeGenOptions::FramePointerKind::None);
  CG.BBSections = "none";
  CG.NoCommon = true;
  CG.setAssignmentTrackingMode(CodeGenOptions::AssignmentTrackingOpts::Enabled);
  CG.MergeAllConstants = false;
  CG.DirectAccessExternalData = !O.PIC;
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
  CG.setExceptionHandling(CodeGenOptions::ExceptionHandlingKind::Wasm);
  if (O.CXX)
    CG.CXXCtorDtorAliases = true;

  DiagnosticOptions &DO = Inv.getDiagnosticOpts();
  DO.Warnings = O.Warnings;
  DO.ErrorLimit = 19;
  DO.ShowOptionNames = true;
  DO.ShowPresumedLoc = true;
  if (O.Colors)
    DO.setShowColors(ShowColorsKind::On);
  DO.ElideType = true;

  AnalyzerOptions &AO = Inv.getAnalyzerOpts();
  AO.ShouldEmitErrorsOnInvalidConfigValue = true;
  AO.InlineMaxStackDepth = 5;
  AO.maxBlockVisitOnPath = 4;

  HeaderSearchOptions &HS = Inv.getHeaderSearchOpts();
  HS.Sysroot = O.Sysroot;
  HS.ResourceDir = O.ResourceDir;
  for (const auto &P : O.Include)
    HS.AddPath(P, frontend::Angled, false, true);
  for (const auto &P : O.ISystem)
    HS.AddPath(P, frontend::System, false, true);
  for (const auto &P : O.ExternCISystem)
    HS.AddPath(P, frontend::ExternCSystem, false, true);
  for (const auto &[Name, File] : O.ModuleFiles)
    HS.PrebuiltModuleFiles[Name] = File;

  FrontendOptions &FO = Inv.getFrontendOpts();
  FO.DisableFree = true;
  FO.ProgramAction = frontend::EmitObj;
  if (O.ModuleInterface) {
    FO.ProgramAction = frontend::GenerateModuleInterface;
    FO.GenReducedBMI = true;
    HS.ModulesSkipDiagnosticOptions = HS.ModulesSkipHeaderSearchPaths = true;
  }
  FO.LLVMArgs = {"-wasm-enable-sjlj", "-wasm-use-legacy-eh=false"};
  if (O.CXX)
    FO.LLVMArgs.insert(FO.LLVMArgs.begin(), "-wasm-enable-eh");
  FO.DashX = InputKind(L);
  FO.OutputFile = O.Output;
  FO.Inputs.push_back(FrontendInputFile(O.Input, InputKind(L)));
}

void scSetLLVMArgs(const CompilerInvocation &Inv) {
  auto &Opts = llvm::cl::getRegisteredOptions();
  for (llvm::StringRef A : Inv.getFrontendOpts().LLVMArgs) {
    auto [Name, Value] = A.drop_front().split('=');
    if (llvm::cl::Option *Opt = Opts.lookup(Name); Opt && !Opt->getNumOccurrences())
      Opt->addOccurrence(1, Name, Value);
  }
}

// Accepts the subset of cc1 flags the engine passes; everything else is fixed.
bool scParseArgs(llvm::ArrayRef<const char *> Args, ScOptions &O) {
  bool Lang = false; // -x: the language, else the extension of the input
  for (size_t i = 0; i < Args.size(); ++i) {
    llvm::StringRef A = Args[i];
    auto Next = [&]() -> const char * { return i + 1 < Args.size() ? Args[++i] : ""; };
    if (A == "-o") O.Output = Next();
    else if (A == "-isysroot") O.Sysroot = Next();
    else if (A == "-resource-dir") O.ResourceDir = Next();
    else if (A == "-internal-isystem") O.ISystem.push_back(Next());
    else if (A == "-internal-externc-isystem") O.ExternCISystem.push_back(Next());
    else if (A == "-mrelocation-model") O.PIC = llvm::StringRef(Next()) == "pic";
    else if (A == "-fvisibility=default") O.VisibilityDefault = true;
    else if (A == "-emit-module-interface" || A == "-fmodules-reduced-bmi") O.ModuleInterface = true;
    else if (A == "-fcolor-diagnostics") O.Colors = true;
    else if (A == "-sc-one-cell") O.OneCell = true;
    else if (A == "-I") O.Include.push_back(Next());
    else if (A == "-x") {
      O.CXX = llvm::StringRef(Next()) == "c++";
      Lang = true;
    }
    else if (A.consume_front("-std=")) O.Std = A.str();
    else if (A.consume_front("-fmodule-file=")) {
      auto [Name, File] = A.split('=');
      O.ModuleFiles.emplace_back(Name.str(), File.str());
    }
    else if (A == "-triple" || A == "-target-cpu" || A == "-ferror-limit" || A == "-pic-level" ||
             A == "-target-feature" || A == "-mllvm")
      Next();
    else if (A.starts_with("-W"))
      O.Warnings.push_back(A.drop_front(2).str());
    else if (!A.starts_with("-")) {
      O.Input = A.str();
      if (!Lang)
        O.CXX = A.ends_with(".cpp") || A.ends_with(".cc") || A.ends_with(".cxx") || A.ends_with(".cppm");
    }
  }
  return !O.Input.empty();
}
