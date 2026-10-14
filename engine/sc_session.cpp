// Candidate A of the REPL (PLANO-REPL.md §3): a clang session in which each
// cell is parsed on top of the ones before, as clang-repl does, but without
// clangInterpreter: a minimal incremental action, a plain CodeGenerator and
// sc's O0 backend (sc_backend.cpp) on the module of each cell.
//
// The core of clang/lib/Interpreter/IncrementalParser.cpp (Parse,
// ParseOrWrapTopLevelDecl, CleanUpPTU) is copied below from llvmorg-23.1.3
// (0d261d1ca552). The class keeps the name IncrementalParser, and the
// unmerger ASTDeclUnmerger, because the AST lets them reach its private
// state. clangInterpreter is not linked.
#include "sc_session.h"
#include "clang/AST/ASTContext.h"
#include "clang/AST/Decl.h"
#include "clang/AST/DeclCXX.h"
#include "clang/AST/DeclContextInternals.h"
#include "clang/AST/DeclTemplate.h"
#include "clang/AST/DeclVisitor.h"
#include "clang/Basic/DiagnosticSema.h"
#include "clang/Basic/TargetInfo.h"
#include "clang/CodeGen/BackendUtil.h"
#include "clang/CodeGen/ModuleBuilder.h"
#include "clang/Frontend/CompilerInstance.h"
#include "clang/Frontend/CompilerInvocation.h"
#include "clang/Frontend/FrontendAction.h"
#include "clang/Lex/HeaderSearch.h"
#include "clang/Lex/Preprocessor.h"
#include "clang/Lex/PreprocessorOptions.h"
#include "clang/Parse/Parser.h"
#include "clang/Sema/Sema.h"
#include "llvm/Analysis/ValueTracking.h"
#include "llvm/IR/InstIterator.h"
#include "llvm/IR/Instructions.h"
#include "llvm/IR/IntrinsicInst.h"
#include "llvm/IR/LLVMContext.h"
#include "llvm/IR/Module.h"
#include "llvm/Support/MemoryBuffer.h"
#include "llvm/Support/VirtualFileSystem.h"
#include "llvm/Support/raw_ostream.h"

using namespace clang;

extern "C" {
void LLVMInitializeWebAssemblyTargetInfo();
void LLVMInitializeWebAssemblyTarget();
void LLVMInitializeWebAssemblyTargetMC();
void LLVMInitializeWebAssemblyAsmPrinter();
void LLVMInitializeWebAssemblyAsmParser();
}

// --- §3.2: one copy of each internal global for the whole session

static bool isInitFunction(llvm::StringRef N) {
  return N.starts_with("__cxx_global_var_init") || N.starts_with("_GLOBAL__sub_I_") ||
         N.starts_with("__stmts__") || N.starts_with("__cxx_global_array_dtor") ||
         N.starts_with("__dtor_");
}

// the globals F writes: stores, the object of a call (a constructor's this)
// and the destination of a memcpy or memset
static void written(llvm::Function &F, llvm::SmallPtrSetImpl<llvm::GlobalValue *> &Out) {
  auto Add = [&](llvm::Value *P) {
    if (auto *G = llvm::dyn_cast<llvm::GlobalVariable>(llvm::getUnderlyingObject(P)))
      Out.insert(G);
  };
  for (llvm::Instruction &I : llvm::instructions(F)) {
    if (auto *S = llvm::dyn_cast<llvm::StoreInst>(&I))
      Add(S->getPointerOperand());
    else if (auto *C = llvm::dyn_cast<llvm::CallBase>(&I)) {
      llvm::Function *Callee = C->getCalledFunction();
      if (Callee && Callee->getName() == "__cxa_atexit")
        continue;
      if (C->arg_size() && C->getArgOperand(0)->getType()->isPointerTy())
        Add(C->getArgOperand(0));
    }
  }
}

void scPromoteInternals(llvm::Module &M, const std::set<std::string> &Known,
                        const std::function<bool(llvm::StringRef)> &IsDecl) {
  llvm::SmallPtrSet<llvm::GlobalValue *, 8> Dropped;
  for (llvm::GlobalValue &GV : M.global_values()) {
    if (GV.isDeclaration() || !GV.hasName() || llvm::isa<llvm::GlobalAlias>(GV))
      continue;
    if (!GV.hasLocalLinkage()) {
      if (!GV.isWeakForLinker() && Known.count(GV.getName().str()))
        Dropped.insert(&GV);
      continue;
    }
    std::string Name = GV.getName().str();
    if (Name[0] == '.' || isInitFunction(Name) || !IsDecl(Name))
      continue;
    std::string New = Name + ".sc";
    GV.setName(New);
    GV.setLinkage(llvm::GlobalValue::ExternalLinkage);
    GV.setVisibility(llvm::GlobalValue::DefaultVisibility);
    GV.setDSOLocal(false); // exported: its address goes through the GOT, as wasm-ld wants
    if (Known.count(New))
      Dropped.insert(&GV); // a copy of one an earlier cell defines
  }
  if (Dropped.empty())
    return;
  // the dynamic initializers that write only dropped copies would run again
  for (llvm::Function &F : M) {
    if (F.isDeclaration() || !F.getName().starts_with("__cxx_global_var_init"))
      continue;
    llvm::SmallPtrSet<llvm::GlobalValue *, 4> W;
    written(F, W);
    if (W.empty() || !llvm::all_of(W, [&](llvm::GlobalValue *G) { return Dropped.count(G); }))
      continue;
    F.deleteBody();
    F.setLinkage(llvm::GlobalValue::InternalLinkage);
    llvm::ReturnInst::Create(M.getContext(), llvm::BasicBlock::Create(M.getContext(), "", &F));
  }
  for (llvm::GlobalValue *GV : Dropped) {
    if (auto *F = llvm::dyn_cast<llvm::Function>(GV))
      F->deleteBody();
    else if (auto *V = llvm::dyn_cast<llvm::GlobalVariable>(GV))
      V->setInitializer(nullptr);
    GV->setLinkage(llvm::GlobalValue::ExternalLinkage);
  }
}

std::vector<std::string> scExports(const llvm::Module &M) {
  std::vector<std::string> R;
  for (const llvm::GlobalValue &GV : M.global_values())
    if (!GV.isDeclaration() && !GV.hasLocalLinkage() && !GV.hasHiddenVisibility() &&
        !GV.getName().starts_with("llvm."))
      R.push_back(GV.getName().str());
  std::sort(R.begin(), R.end());
  return R;
}

// --- the copy of clang-repl's IncrementalParser (llvmorg-23.1.3)

namespace clang {

class IncrementalParser {
public:
  Sema &S;
  std::unique_ptr<Parser> P;
  ASTConsumer *Consumer = nullptr;
  CodeGenerator *CG;
  llvm::LLVMContext &Ctx;
  unsigned InputCount = 0;

  IncrementalParser(CompilerInstance &Instance, CodeGenerator *CG, llvm::LLVMContext &Ctx)
      : S(Instance.getSema()), CG(CG), Ctx(Ctx) {
    Consumer = &S.getASTConsumer();
    P.reset(new Parser(S.getPreprocessor(), S, /*SkipBodies=*/false));
    if (ExternalASTSource *External = S.getASTContext().getExternalSource())
      External->StartTranslationUnit(Consumer);
    P->Initialize();
  }

  // §3.3: the module goes before the AST, so what the CodeGen saw of the
  // cell does not leak into the next one
  void Fail(TranslationUnitDecl *TU) {
    if (!CG->GetModule())
      CG->StartModule("sc_cell", Ctx);
    CleanUpPTU(TU);
    DiagnosticsEngine &Diags = S.getDiagnostics();
    Diags.Reset(/*soft=*/true);
    Diags.getClient()->clear();
  }

  TranslationUnitDecl *ParseOrWrapTopLevelDecl() {
    Sema::GlobalEagerInstantiationScope GlobalInstantiations(S, /*Enabled=*/true,
                                                             /*AtEndOfTU=*/true);
    Sema::LocalEagerInstantiationScope LocalInstantiations(S, /*AtEndOfTU=*/true);

    // Add a new PTU.
    ASTContext &C = S.getASTContext();
    C.addTranslationUnitDecl();

    // Skip previous eof due to last incremental input.
    if (P->getCurToken().is(tok::annot_repl_input_end)) {
      P->ConsumeAnyToken();
      P->ExitScope();
      S.CurContext = nullptr;
      // Start a new PTU.
      P->EnterScope(Scope::DeclScope);
      S.ActOnTranslationUnitScope(P->getCurScope());
    }

    Parser::DeclGroupPtrTy ADecl;
    Sema::ModuleImportState ImportState;
    for (bool AtEOF = P->ParseFirstTopLevelDecl(ADecl, ImportState); !AtEOF;
         AtEOF = P->ParseTopLevelDecl(ADecl, ImportState)) {
      if (ADecl && !Consumer->HandleTopLevelDecl(ADecl.get()))
        break;
    }

    DiagnosticsEngine &Diags = S.getDiagnostics();
    if (Diags.hasErrorOccurred()) {
      Consumer->HandleTranslationUnit(C); // drops the module
      Fail(C.getTranslationUnitDecl());
      return nullptr;
    }

    // Process any TopLevelDecls generated by #pragma weak.
    for (Decl *D : S.WeakTopLevelDecls()) {
      DeclGroupRef DGR(D);
      Consumer->HandleTopLevelDecl(DGR);
    }

    LocalInstantiations.perform();
    GlobalInstantiations.perform();

    Consumer->HandleTranslationUnit(C);
    if (Diags.hasErrorOccurred()) { // in the instantiations or the codegen
      Fail(C.getTranslationUnitDecl());
      return nullptr;
    }
    return C.getTranslationUnitDecl();
  }

  TranslationUnitDecl *Parse(llvm::StringRef Input) {
    Preprocessor &PP = S.getPreprocessor();
    std::string SourceName = "cell_" + std::to_string(++InputCount);

    // Create an uninitialized memory buffer, copy code in and append "\n"
    size_t InputSize = Input.size();
    std::unique_ptr<llvm::MemoryBuffer> MB(
        llvm::WritableMemoryBuffer::getNewUninitMemBuffer(InputSize + 1, SourceName));
    char *MBStart = const_cast<char *>(MB->getBufferStart());
    memcpy(MBStart, Input.data(), InputSize);
    MBStart[InputSize] = '\n';

    SourceManager &SM = S.getSourceManager();
    SourceLocation NewLoc = SM.getLocForStartOfFile(SM.getMainFileID());
    FileEntryRef FE = SM.getFileManager().getVirtualFileRef(SourceName, InputSize, 0);
    SM.overrideFileContents(FE, std::move(MB));
    PP.getHeaderSearchInfo().getFileInfo(FE);
    FileID FID = SM.createFileID(FE, NewLoc, SrcMgr::C_User);
    if (PP.EnterSourceFile(FID, /*DirLookup=*/nullptr, NewLoc))
      return nullptr;

    TranslationUnitDecl *TU = ParseOrWrapTopLevelDecl();
    if (!TU)
      return nullptr;
    Token AssertTok;
    PP.Lex(AssertTok);
    return TU;
  }

  void withdrawMostRecentTU(TranslationUnitDecl *MostRecentTU);
  void CleanUpPTU(TranslationUnitDecl *MostRecentTU);
};

void IncrementalParser::withdrawMostRecentTU(TranslationUnitDecl *MostRecentTU) {
  TranslationUnitDecl *Prev = MostRecentTU->getPreviousDecl();
  if (!Prev)
    return;
  // Rebuild A -> ... -> Prev -> MostRecentTU as A -> ... -> Prev.
  MostRecentTU->getFirstDecl()->RedeclLink.setLatest(Prev);
  // getTranslationUnitDecl() requires the active unit to be the latest one.
  ASTContext &C = S.getASTContext();
  if (C.TraversalScope.size() == 1 && C.TraversalScope.back() == MostRecentTU)
    C.TraversalScope = {Prev};
  C.TUDecl = Prev;
}

/// Removes decls introduced in the discarding PTU and restores the
/// redeclaration chain to previous state.
class ASTDeclUnmerger : public DeclVisitor<ASTDeclUnmerger> {
  Sema &S;
  TranslationUnitDecl *DiscardedTU;

  template <typename DeclT> void withdraw(Redeclarable<DeclT> *DBase) {
    if (NamedDecl *Prev = findSurvivor(static_cast<DeclT *>(DBase)))
      unlinkRedeclChain(S.getASTContext(), DBase, Prev);
  }

  NamedDecl *findSurvivor(NamedDecl *D) const {
    for (Decl *Prev = D->getPreviousDecl(); Prev; Prev = Prev->getPreviousDecl())
      if (Prev->getTranslationUnitDecl() != DiscardedTU)
        return dyn_cast<NamedDecl>(Prev);
    return nullptr;
  }

  template <typename DeclT>
  void unlinkRedeclChain(ASTContext &C, Redeclarable<DeclT> *DBase, NamedDecl *PrevND) {
    auto *Latest = static_cast<DeclT *>(DBase);
    auto *Survivor = cast<DeclT>(PrevND);
    Latest->getFirstDecl()->RedeclLink.setLatest(Survivor);
    for (DeclT *Dead = Latest; Dead != Survivor;) {
      DeclT *Next = Dead->getPreviousDecl();
      Dead->First = Dead;
      Dead->RedeclLink = Redeclarable<DeclT>::LatestDeclLink(C);
      Dead = Next;
    }
  }

  void removeFromLookups(NamedDecl *D) {
    if (D->getDeclName().isEmpty())
      return;
    if (D->getDeclName().isIdentifier() && D->getDeclName().getFETokenInfo() &&
        !D->getLangOpts().ObjC && !D->getLangOpts().CPlusPlus)
      S.IdResolver.RemoveDecl(D);
    ExternCContextDecl *ECCD = S.getASTContext().getExternCContextDecl();
    if (StoredDeclsMap *Map = ECCD->getPrimaryContext()->getLookupPtr()) {
      auto It = Map->find(D->getDeclName());
      if (It != Map->end())
        It->second.remove(D);
    }
  }

  void VisitDeclContext(DeclContext *DC) {
    llvm::SmallVector<Decl *, 8> Members(DC->decls());
    llvm::SmallVector<NamedDecl *, 8> Survivors;
    for (Decl *M : Members) {
      if (auto *ND = dyn_cast<NamedDecl>(M))
        if (NamedDecl *Prev = findSurvivor(ND))
          Survivors.push_back(Prev);
      Visit(M);
      DC->removeDecl(M);
      if (auto *ND = dyn_cast<NamedDecl>(M))
        removeFromLookups(ND);
    }
    DeclContext *Primary = DC->getPrimaryContext();
    for (NamedDecl *Prev : Survivors)
      Primary->makeDeclVisibleInContext(Prev);
  }

public:
  ASTDeclUnmerger(Sema &S, TranslationUnitDecl *DiscardedTU) : S(S), DiscardedTU(DiscardedTU) {}

  void VisitDecl(Decl *D) {
    if (auto *DC = dyn_cast<DeclContext>(D))
      VisitDeclContext(DC);
  }
  void VisitFunctionDecl(FunctionDecl *D) { withdraw(D); }
  void VisitNamespaceAliasDecl(NamespaceAliasDecl *D) { withdraw(D); }
  void VisitTypedefNameDecl(TypedefNameDecl *D) { withdraw(D); }
  void VisitUsingShadowDecl(UsingShadowDecl *D) { withdraw(D); }
  void VisitVarDecl(VarDecl *D) { withdraw(D); }

  void VisitTagDecl(TagDecl *D) {
    NamedDecl *Prev = findSurvivor(D);
    if (!Prev)
      return;
    unlinkRedeclChain(S.getASTContext(), D, Prev);
    auto *RD = dyn_cast<CXXRecordDecl>(Prev);
    if (!RD)
      return;
    if (CXXRecordDecl *Def = RD->getDefinition(); Def && Def->getTranslationUnitDecl() == DiscardedTU)
      for (auto *R : RD->redecls())
        cast<CXXRecordDecl>(R)->DefinitionData = nullptr;
  }

  void VisitRedeclarableTemplateDecl(RedeclarableTemplateDecl *D) {
    withdraw(D);
    Visit(D->getTemplatedDecl());
  }

  void VisitNamespaceDecl(NamespaceDecl *D) {
    VisitDeclContext(D);
    withdraw(D);
  }

  void VisitTranslationUnitDecl(TranslationUnitDecl *D) { VisitDeclContext(D); }
};

void IncrementalParser::CleanUpPTU(TranslationUnitDecl *MostRecentTU) {
  ASTDeclUnmerger(S, MostRecentTU).Visit(MostRecentTU);
  withdrawMostRecentTU(MostRecentTU);
}

} // namespace clang

// --- the session

namespace {
// IncrementalAction without the wrapped EmitLLVMOnlyAction: its consumer
// would run the backend with Backend_EmitNothing, which sc_backend rejects
class SessionAction : public ASTFrontendAction {
  llvm::LLVMContext &Ctx;

public:
  CodeGenerator *CG = nullptr;
  SessionAction(llvm::LLVMContext &Ctx) : Ctx(Ctx) {}
  std::unique_ptr<ASTConsumer> CreateASTConsumer(CompilerInstance &CI, llvm::StringRef InFile) override {
    auto C = CreateLLVMCodeGen(CI, "sc_cell_0", Ctx);
    CG = C.get();
    return C;
  }
  TranslationUnitKind getTranslationUnitKind() override { return TU_Incremental; }
  bool hasIRSupport() const override { return true; }
  void ExecuteAction() override {
    ASTFrontendAction::ExecuteAction();
    getCompilerInstance().getSema().CurContext = nullptr;
  }
  void EndSourceFile() override {} // the session lives on
};
} // namespace

struct ScSession::Impl {
  llvm::LLVMContext Ctx;
  std::unique_ptr<CompilerInstance> CI;
  std::unique_ptr<SessionAction> Act;
  std::unique_ptr<IncrementalParser> IP;
  std::unique_ptr<llvm::Module> Initial; // the CodeGen keeps references to it
  std::set<std::string> Known; // what the cells so far export
  unsigned N = 0;
};

static constexpr const char *MainFile = "<<< inputs >>>";

std::unique_ptr<ScSession> ScSession::create(ScOptions O) {
  LLVMInitializeWebAssemblyTargetInfo();
  LLVMInitializeWebAssemblyTarget();
  LLVMInitializeWebAssemblyTargetMC();
  LLVMInitializeWebAssemblyAsmPrinter();
  LLVMInitializeWebAssemblyAsmParser();

  O.Input = MainFile;
  O.PIC = O.VisibilityDefault = true;
  auto Inv = std::make_shared<CompilerInvocation>();
  scBuildInvocation(*Inv, O);
  scSetLLVMArgs(*Inv);
  // §3.5
  Inv->getLangOpts().IncrementalExtensions = true;
  Inv->getCodeGenOpts().DisableFree = false;
  Inv->getCodeGenOpts().ClearASTBeforeBackend = false;
  Inv->getFrontendOpts().DisableFree = false;
  Inv->getPreprocessorOpts().addRemappedFile(MainFile, llvm::MemoryBuffer::getMemBuffer("").release());

  std::unique_ptr<ScSession> S(new ScSession);
  S->I = std::make_unique<Impl>();
  Impl &I = *S->I;
  I.CI = std::make_unique<CompilerInstance>(std::move(Inv));
  I.CI->createVirtualFileSystem(llvm::vfs::getRealFileSystem());
  I.CI->createDiagnostics();
  I.Act = std::make_unique<SessionAction>(I.Ctx);
  if (!I.CI->ExecuteAction(*I.Act) || I.CI->getDiagnostics().hasErrorOccurred())
    return nullptr;
  I.IP = std::make_unique<IncrementalParser>(*I.CI, I.Act->CG, I.Ctx);
  I.Initial = I.Act->CG->ReleaseModule();
  I.Act->CG->StartModule("sc_cell_1", I.Ctx);
  // as clang-repl: value printing would leave unused expressions
  I.CI->getDiagnostics().setSeverity(diag::warn_unused_expr, diag::Severity::Ignored, SourceLocation());
  return S;
}

ScSession::~ScSession() = default;

ScStatus ScSession::cell(llvm::StringRef Src, bool Backend, std::string &Obj,
                         std::vector<std::string> &Syms) {
  Impl &I = *this->I;
  CompilerInstance &CI = *I.CI;
  Obj.clear();
  Syms.clear();
  if (!I.IP->Parse(Src))
    return ScError;
  CodeGenerator *CG = I.Act->CG;
  std::unique_ptr<llvm::Module> M = CG->ReleaseModule();
  CG->StartModule("sc_cell_" + std::to_string(++I.N + 1), I.Ctx);
  if (!M)
    return ScLost;
  scPromoteInternals(*M, I.Known, [&](llvm::StringRef Name) { return CG->GetDeclForMangledName(Name); });
  Syms = scExports(*M);
  I.Known.insert(Syms.begin(), Syms.end());
  if (!Backend)
    return ScOk;
  llvm::SmallString<0> Buf;
  emitBackendOutput(CI, CI.getCodeGenOpts(), CI.getTarget().getDataLayoutString(), M.get(),
                    Backend_EmitObj, CI.getVirtualFileSystemPtr(),
                    std::make_unique<llvm::raw_svector_ostream>(Buf));
  if (CI.getDiagnostics().hasErrorOccurred())
    return ScLost;
  Obj.assign(Buf.begin(), Buf.end());
  return ScOk;
}
