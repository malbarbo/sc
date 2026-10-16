// Builds the clang CompilerInvocation for sc directly, without the cc1 option
// parser (CompilerInvocation::CreateFromArgs and the option tables).
//
// The result must match what `clang -cc1` produces for the flags the engine
// always uses; checkinv.cpp (native) compares both via generateCC1CommandLine.
#include "sc_inv.h"
#include "clang/Basic/LangOptions.h"
#include "clang/Basic/LangStandard.h"
#include "clang/Basic/Sanitizers.h"
#include "clang/Basic/TargetOptions.h"
#include "clang/Lex/HeaderSearchOptions.h"
#include "clang/Lex/PreprocessorOptions.h"
#include "clang/Frontend/CompilerInvocation.h"
#include "clang/StaticAnalyzer/Core/AnalyzerOptions.h"
#include "clang/Frontend/FrontendOptions.h"
#include "llvm/IR/DebugInfoMetadata.h"
#include "llvm/ADT/StringSet.h"
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
  LO.GPUDefaultStream = LangOptions::GPUDefaultStreamKind::Legacy;
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
  if (O.PatternInit)
    LO.setTrivialAutoVarInit(LangOptions::TrivialAutoVarInitKind::Pattern);
  auto Kinds = [](const std::vector<std::string> &Names, SanitizerSet &S) {
    for (const auto &N : Names)
      S.set(parseSanitizerValue(N, /*AllowGroups=*/false), true);
  };
  Kinds(O.Sanitize, LO.Sanitize);

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
  CG.SanitizeAddressUseOdrIndicator = !O.NoOdrIndicator;
  CG.SanitizeMemoryParamRetval = !O.NoParamRetval;
  Kinds(O.SanitizeTrap, CG.SanitizeTrap);
  Kinds(O.SanitizeMerge, CG.SanitizeMergeHandlers);
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
  if (O.LineTables) { // -gline-tables-only
    CG.setDebugInfo(llvm::codegenoptions::DebugLineTablesOnly);
    CG.DwarfVersion = 4;
    CG.setDebuggerTuning(llvm::DebuggerKind::GDB);
  }
  // fields without a default, which the marshalling always sets
  CG.SanitizeKcfiHash = llvm::KCFIHashAlgorithm::xxHash64;

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

  for (const auto &M : O.Macros)
    Inv.getPreprocessorOpts().addMacroDef(M);

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
  FO.PrintSupportedCPUs = FO.PrintSupportedExtensions = FO.PrintEnabledExtensions = false;
  FO.ASTDumpDeclTypes = FO.IsSystemModule = false;
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

// Accepts the cc1 flags the engine is given (scripts/flags.py) and turns down
// any other: those that are fixed must have their fixed value.
bool scParseArgs(llvm::ArrayRef<const char *> Args, ScOptions &O, std::string &Error) {
  static const llvm::StringSet<> Fixed = {
      "-emit-obj", "-disable-free", "-clear-ast-before-backend", "-disable-llvm-verifier",
      "-discard-value-names", "-mframe-pointer=none", "-ffp-contract=on", "-fno-rounding-math",
      "-fvisibility=hidden", "-fgnuc-version=4.2.1", "-exception-model=wasm", "-mconstructor-aliases",
      "-fdeprecated-macro", "-fno-implicit-modules", "-fskip-odr-check-in-gmf", "-fcxx-exceptions",
      "-fexceptions", "-dwarf-version=4", "-debugger-tuning=gdb"};
  static const std::pair<llvm::StringRef, llvm::StringSet<>> FixedValue[] = {
      {"-triple", {"wasm32-unknown-wasip1"}},
      {"-target-cpu", {"generic"}},
      {"-ferror-limit", {"19"}},
      {"-pic-level", {"2"}},
      {"-target-feature", {"+exception-handling", "+multivalue", "+reference-types", "+mutable-globals"}},
      {"-mllvm", {"-wasm-enable-sjlj", "-wasm-use-legacy-eh=false", "-wasm-enable-eh"}},
      {"-mrelocation-model", {"static", "pic"}}};
  bool Lang = false; // -x: the language, else the extension of the input
  for (size_t i = 0; i < Args.size(); ++i) {
    llvm::StringRef A = Args[i];
    auto Next = [&]() -> const char * { return i + 1 < Args.size() ? Args[++i] : ""; };
    auto *FV = llvm::find_if(FixedValue, [&](auto &F) { return F.first == A; });
    if (FV != std::end(FixedValue)) {
      llvm::StringRef V = Next();
      if (!FV->second.contains(V)) {
        Error = ("unsupported value of " + A + ": '" + V + "'").str();
        return false;
      }
      if (A == "-mrelocation-model") O.PIC = V == "pic";
    }
    else if (Fixed.contains(A)) continue;
    else if (A == "-o") O.Output = Next();
    else if (A == "-isysroot") O.Sysroot = Next();
    else if (A == "-resource-dir") O.ResourceDir = Next();
    else if (A == "-internal-isystem") O.ISystem.push_back(Next());
    else if (A == "-internal-externc-isystem") O.ExternCISystem.push_back(Next());
    else if (A == "-fvisibility=default") O.VisibilityDefault = true;
    else if (A == "-emit-module-interface" || A == "-fmodules-reduced-bmi") O.ModuleInterface = true;
    else if (A == "-fcolor-diagnostics") O.Colors = true;
    else if (A == "-sc-one-cell") O.OneCell = true;
    else if (A == "-debug-info-kind=line-tables-only") O.LineTables = true;
    else if (A == "-ftrivial-auto-var-init=pattern") O.PatternInit = true;
    else if (A == "-fno-sanitize-memory-param-retval") O.NoParamRetval = true;
    else if (A == "-fno-sanitize-address-use-odr-indicator") O.NoOdrIndicator = true;
    else if (A == "-I") O.Include.push_back(Next());
    else if (A == "-D") O.Macros.push_back(Next());
    else if (A.starts_with("-fsanitize=") || A.starts_with("-fsanitize-trap=") || A.starts_with("-fsanitize-merge=")) {
      auto [Flag, List] = A.split('=');
      auto &To = Flag == "-fsanitize" ? O.Sanitize : Flag == "-fsanitize-trap" ? O.SanitizeTrap : O.SanitizeMerge;
      llvm::SmallVector<llvm::StringRef> Names;
      List.split(Names, ',', -1, false);
      for (llvm::StringRef N : Names) {
        if (parseSanitizerValue(N, /*AllowGroups=*/false) == SanitizerMask()) {
          Error = ("unsupported value of " + Flag + "=: '" + N + "'").str();
          return false;
        }
        To.push_back(N.str());
      }
    }
    else if (A == "-x") {
      llvm::StringRef X = Next();
      if (X != "c" && X != "c++") {
        Error = ("unsupported language: '" + X + "'").str();
        return false;
      }
      O.CXX = X == "c++";
      Lang = true;
    }
    else if (A.consume_front("-std=")) {
      if (LangStandard::getLangKind(A) == LangStandard::lang_unspecified) {
        Error = ("unknown standard: '" + A + "'").str();
        return false;
      }
      O.Std = A.str();
    }
    else if (A.consume_front("-fmodule-file=")) {
      auto [Name, File] = A.split('=');
      O.ModuleFiles.emplace_back(Name.str(), File.str());
    }
    else if (A.starts_with("-W"))
      O.Warnings.push_back(A.drop_front(2).str());
    else if (!A.starts_with("-")) {
      O.Input = A.str();
      if (!Lang)
        O.CXX = A.ends_with(".cpp") || A.ends_with(".cc") || A.ends_with(".cxx") || A.ends_with(".cppm");
    }
    else {
      Error = ("unknown argument: '" + A + "'").str();
      return false;
    }
  }
  return true;
}
