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
#include "sc.h"
#include "clang/AST/ASTContext.h"
#include "clang/AST/Decl.h"
#include "clang/AST/DeclCXX.h"
#include "clang/AST/DeclContextInternals.h"
#include "clang/AST/DeclTemplate.h"
#include "clang/AST/DeclVisitor.h"
#include "clang/AST/GlobalDecl.h"
#include "clang/Basic/DiagnosticFrontend.h"
#include "clang/Basic/DiagnosticParse.h"
#include "clang/Basic/TargetInfo.h"
#include "clang/CodeGen/BackendUtil.h"
#include "clang/CodeGen/ModuleBuilder.h"
#include "clang/Frontend/CompilerInstance.h"
#include "clang/Frontend/CompilerInvocation.h"
#include "clang/Frontend/FrontendAction.h"
#include "clang/Frontend/TextDiagnosticPrinter.h"
#include "clang/Lex/HeaderSearch.h"
#include "clang/Lex/Lexer.h"
#include "clang/Lex/Preprocessor.h"
#include "clang/Lex/PreprocessorOptions.h"
#include "clang/Parse/Parser.h"
#include "clang/Sema/Lookup.h"
#include "clang/Sema/Sema.h"
#include "llvm/Analysis/ValueTracking.h"
#include "llvm/IR/DiagnosticInfo.h"
#include "llvm/IR/DiagnosticPrinter.h"
#include "llvm/IR/InstIterator.h"
#include "llvm/IR/Instructions.h"
#include "llvm/IR/IntrinsicInst.h"
#include "llvm/IR/LLVMContext.h"
#include "llvm/IR/Module.h"
#include "llvm/Support/MemoryBuffer.h"
#include "llvm/Support/VirtualFileSystem.h"
#include "llvm/Support/raw_ostream.h"
#include <map>

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

static void promote(llvm::GlobalValue &GV) {
  GV.setName(GV.getName() + ".sc");
  GV.setLinkage(llvm::GlobalValue::ExternalLinkage);
  GV.setVisibility(llvm::GlobalValue::DefaultVisibility);
  GV.setDSOLocal(false); // exported: its address goes through the GOT, as wasm-ld wants
}

void scPromoteInternals(llvm::Module &M, const std::set<std::string> &Known,
                        const std::function<const clang::Decl *(llvm::StringRef)> &DeclOf) {
  llvm::SmallPtrSet<llvm::GlobalValue *, 8> Dropped;
  for (llvm::GlobalValue &GV : M.global_values()) {
    if (!GV.hasName() || llvm::isa<llvm::GlobalAlias>(GV) || GV.getName().starts_with("llvm."))
      continue;
    if (GV.isDeclaration()) { // a static used here and defined in a later cell
      auto *D = llvm::dyn_cast_or_null<clang::NamedDecl>(DeclOf(GV.getName()));
      if (D && !D->isExternallyVisible())
        promote(GV);
      continue;
    }
    if (!GV.hasLocalLinkage()) {
      if (!GV.isWeakForLinker() && Known.count(GV.getName().str()))
        Dropped.insert(&GV);
      continue;
    }
    std::string Name = GV.getName().str();
    if (Name[0] == '.' || isInitFunction(Name) || !DeclOf(Name))
      continue;
    promote(GV);
    if (Known.count(Name + ".sc"))
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

// A cell's declaration of a function an earlier cell defines takes the type
// of the definition: a call to a function without a prototype (C before C23)
// declares it variadic, and in one translation unit the definition would
// replace that declaration. The backend fixes the calls, as it does there.
static void retypeDecls(llvm::Module &M, const std::map<std::string, llvm::FunctionType *> &Types) {
  for (llvm::Function &F : llvm::make_early_inc_range(M)) {
    if (!F.isDeclaration() || F.isIntrinsic())
      continue;
    auto It = Types.find(F.getName().str());
    if (It == Types.end() || It->second == F.getFunctionType())
      continue;
    std::string Name = F.getName().str();
    F.setName(Name + ".sc.old");
    llvm::Function *New = llvm::Function::Create(It->second, F.getLinkage(), Name, M);
    New->copyAttributesFrom(&F);
    New->setAttributes({}); // the parameters differ
    F.replaceAllUsesWith(New);
    F.eraseFromParent();
  }
}

std::vector<std::string> scExports(const llvm::Module &M) {
  std::vector<std::string> R;
  for (const llvm::GlobalValue &GV : M.global_values())
    if (!GV.isDeclaration() && !GV.hasLocalLinkage() && !GV.hasAvailableExternallyLinkage() &&
        !GV.hasHiddenVisibility() && !GV.getName().starts_with("llvm."))
      R.push_back(GV.getName().str());
  std::sort(R.begin(), R.end());
  return R;
}

// --- the value of a cell that ends in an expression without its `;` (sc.h)

static const char *ShowPrelude = "void __sc_show_int(const char *, int, long long);\n"
                                 "void __sc_show_float(const char *, int, double);\n";

// char, through typedefs and qualifiers
static bool isChar(QualType T) {
  return T->isSpecificBuiltinType(BuiltinType::Char_S) || T->isSpecificBuiltinType(BuiltinType::Char_U);
}

// T with the names a typedef at namespace scope gives it (size_t,
// std::string), and the rest canonical: what a class or a template writes
// inside (std::string::size_type, basic_string<_CharT>) says little outside
static QualType plain(ASTContext &C, QualType T) {
  SplitQualType Q = T.split();
  if (auto *TT = dyn_cast<TypedefType>(Q.Ty))
    return TT->getDecl()->getDeclContext()->isRecord() ? plain(C, C.getQualifiedType(TT->desugar(), Q.Quals)) : T;
  if (isa<PredefinedSugarType>(Q.Ty)) // __size_t
    return T;
  if (auto *P = dyn_cast<PointerType>(Q.Ty))
    return C.getQualifiedType(C.getPointerType(plain(C, P->getPointeeType())), Q.Quals);
  return C.getQualifiedType(Q.Ty->getCanonicalTypeInternal(), Q.Quals);
}

// T as clang writes it (plain); the type of sizeof, __size_t, is size_t
static std::string typeName(Sema &S, QualType T) {
  std::string Name = plain(S.getASTContext(), T).getAsString(S.getPrintingPolicy());
  if (isa<PredefinedSugarType>(T))
    Name = llvm::StringRef(Name).drop_while([](char c) { return c == '_'; }).str();
  return Name;
}

// The call that shows the value of E, or null if E is not shown.
static Expr *show(Sema &S, Expr *E) {
  ASTContext &C = S.getASTContext();
  auto *EWC = dyn_cast<ExprWithCleanups>(E); // the temporaries, put back on the call
  if (EWC)
    E = EWC->getSubExpr();
  QualType T = E->IgnoreImpCasts()->getType().getUnqualifiedType();
  QualType D = T.getCanonicalType().getUnqualifiedType();
  int Kind;
  if (D->isBooleanType())
    Kind = SC_SHOW_BOOL;
  else if (isChar(D))
    Kind = SC_SHOW_CHAR;
  else if (D->isIntegralOrEnumerationType() && C.getTypeSize(D) <= 64)
    Kind = D->isSignedIntegerOrEnumerationType() ? SC_SHOW_SIGNED : SC_SHOW_UNSIGNED;
  else if (D->isRealFloatingType())
    Kind = D->isSpecificBuiltinType(BuiltinType::Float) ? SC_SHOW_FLOAT : SC_SHOW_DOUBLE;
  else if (D->isNullPtrType())
    Kind = SC_SHOW_NULLPTR;
  else if (D->isPointerType())
    Kind = isChar(D->getPointeeType()) ? SC_SHOW_STRING : SC_SHOW_POINTER;
  else if (D->isArrayType() && isChar(C.getAsArrayType(D)->getElementType()))
    Kind = SC_SHOW_STRING;
  else
    return nullptr;
  bool Float = Kind == SC_SHOW_FLOAT || Kind == SC_SHOW_DOUBLE;

  LookupResult R(S, &C.Idents.get(Float ? "__sc_show_float" : "__sc_show_int"), SourceLocation(),
                 Sema::LookupOrdinaryName);
  S.LookupQualifiedName(R, C.getTranslationUnitDecl());
  if (!R.isSingleResult())
    return nullptr;
  CXXScopeSpec SS;
  ExprResult Fn = S.BuildDeclarationNameExpr(SS, R, /*NeedsADL=*/false);
  SourceLocation Loc = E->getBeginLoc();
  std::string Name = typeName(S, T);
  Expr *Type = StringLiteral::Create(C, Name, StringLiteralKind::Ordinary, false,
                                     C.getStringLiteralArrayType(C.CharTy, Name.size()), Loc);
  Expr *KindE = IntegerLiteral::Create(C, llvm::APInt(C.getIntWidth(C.IntTy), Kind), C.IntTy, Loc);
  // nullptr does not convert to an integer in C
  ExprResult V = Kind == SC_SHOW_NULLPTR
                     ? IntegerLiteral::Create(C, llvm::APInt(C.getIntWidth(C.LongLongTy), 0), C.LongLongTy, Loc)
                     : S.BuildCStyleCastExpr(Loc, C.getTrivialTypeSourceInfo(Float ? C.DoubleTy : C.LongLongTy, Loc),
                                             Loc, E);
  if (Fn.isInvalid() || V.isInvalid())
    return nullptr;
  Expr *Args[] = {Type, KindE, V.get()};
  ExprResult Call = S.ActOnCallExpr(nullptr, Fn.get(), Loc, Args, E->getEndLoc());
  if (!Call.isInvalid())
    Call = S.ActOnFinishFullExpr(Call.get(), /*DiscardedValue=*/false);
  if (Call.isInvalid())
    return nullptr;
  if (EWC && !isa<ExprWithCleanups>(Call.get()))
    return ExprWithCleanups::Create(C, Call.get(), EWC->cleanupsHaveSideEffects(), EWC->getObjects());
  return Call.get();
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
  FileID LastFID; // the last cell's
  SourceLocation CellEnd; // where the last cell's last token ends
  std::function<void()> BeforeEnd; // before the CodeGen ends the cell's module
  std::string *TypeOf = nullptr;   // ScSession::type: the cell's type goes here
  bool OneCell = false;            // no cell comes after (-sc-one-cell): the cell is a program, and its value is not shown

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
    DiagnosticsEngine &Diags = S.getDiagnostics();
    if (TypeOf) {
      // one expression without its `;`, which the CodeGen does not see
      unsigned Decls = 0;
      for (bool AtEOF = P->ParseFirstTopLevelDecl(ADecl, ImportState); !AtEOF;
           AtEOF = P->ParseTopLevelDecl(ADecl, ImportState))
        if (ADecl && !Diags.hasErrorOccurred())
          for (Decl *D : ADecl.get()) {
            auto *TLSD = dyn_cast<TopLevelStmtDecl>(D);
            auto *E = TLSD && TLSD->isSemiMissing() ? dyn_cast_or_null<Expr>(TLSD->getStmt()) : nullptr;
            if (++Decls == 1 && E)
              *TypeOf = typeName(S, E->IgnoreImplicit()->getType());
            else // after the first error, the loop parses on to the end
              Diags.Report(D->getBeginLoc(), diag::err_expected_expression);
          }
      if (!Decls && !Diags.hasErrorOccurred())
        Diags.Report(S.getSourceManager().getLocForStartOfFile(LastFID), diag::err_expected_expression);
      if (Diags.hasErrorOccurred())
        TypeOf->clear();
      // what Sema handed to the CodeGen on the way (an implicit member's
      // definition) goes with the module
      std::unique_ptr<llvm::Module> M = CG->ReleaseModule();
      Fail(C.getTranslationUnitDecl());
      return nullptr;
    }
    for (bool AtEOF = P->ParseFirstTopLevelDecl(ADecl, ImportState); !AtEOF;
         AtEOF = P->ParseTopLevelDecl(ADecl, ImportState)) {
      if (ADecl && !Diags.hasErrorOccurred())
        for (Decl *D : ADecl.get())
          if (auto *TLSD = dyn_cast<TopLevelStmtDecl>(D); TLSD && TLSD->isSemiMissing())
            if (auto *E = dyn_cast_or_null<Expr>(TLSD->getStmt()))
              if (Expr *Call = OneCell ? nullptr : show(S, E))
                TLSD->setStmt(Call);
      // a later cell may use or define what this one leaves unused or
      // undefined, so the cell's end of translation unit does not say so
      if (!OneCell) {
        S.UnusedFileScopedDecls.erase(S.UnusedFileScopedDecls.begin(nullptr, true), S.UnusedFileScopedDecls.end());
        S.UndefinedButUsed.clear();
      }
      if (ADecl && !Consumer->HandleTopLevelDecl(ADecl.get()))
        break;
    }

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

    if (BeforeEnd)
      BeforeEnd();
    Consumer->HandleTranslationUnit(C);
    if (Diags.hasErrorOccurred()) { // in the instantiations or the codegen
      Fail(C.getTranslationUnitDecl());
      return nullptr;
    }
    return C.getTranslationUnitDecl();
  }

  // Prelude: declarations of sc's own, which no diagnostic shows
  TranslationUnitDecl *Parse(llvm::StringRef Input, bool Prelude = false) {
    Preprocessor &PP = S.getPreprocessor();
    std::string SourceName = Prelude ? "<sc prelude>" : "cell_" + std::to_string(++InputCount);

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
    FileID FID = SM.createFileID(FE, NewLoc, Prelude ? SrcMgr::C_System : SrcMgr::C_User);
    LastFID = FID;
    CellEnd = SM.getLocForStartOfFile(FID);
    Lexer L(FID, SM.getBufferOrFake(FID), SM, S.getLangOpts());
    Token T;
    for (L.LexFromRawLexer(T); T.isNot(tok::eof); L.LexFromRawLexer(T))
      CellEnd = T.getEndLoc();
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
// The backend's errors and warnings (an unsupported builtin) go to clang's
// diagnostics, as CodeGenAction's handler sends them in a one-shot compile
class BackendDiags : public llvm::DiagnosticHandler {
  DiagnosticsEngine &Diags;

public:
  BackendDiags(DiagnosticsEngine &Diags) : Diags(Diags) {}
  bool handleDiagnostics(const llvm::DiagnosticInfo &DI) override {
    if (DI.getSeverity() != llvm::DS_Error && DI.getSeverity() != llvm::DS_Warning)
      return true;
    std::string Msg;
    llvm::raw_string_ostream OS(Msg);
    llvm::DiagnosticPrinterRawOStream DP(OS);
    DI.print(DP);
    Diags.Report(DI.getSeverity() == llvm::DS_Error ? diag::err_fe_backend_unsupported
                                                    : diag::warn_fe_backend_unsupported)
        << Msg;
    return true;
  }
};

static constexpr const char *MainFile = "<<< inputs >>>";

// The diagnostics go on to the printer, and the first error of a cell is
// kept: one at the cell's end says the cell stops short (ScSession::cell).
// Each cell is a file the empty main file includes: the printer's "In file
// included from" the main file is left out, and the main file's one token,
// the end of the input, stands for the end of the cell
class FirstError : public DiagnosticConsumer {
  std::string Buf;
  llvm::raw_string_ostream BufOS{Buf}; // the printer's output, before OS
  std::unique_ptr<DiagnosticConsumer> Printer;
  llvm::raw_ostream &OS;

public:
  const IncrementalParser *IP = nullptr;
  bool Seen = false;
  SourceLocation Loc;
  FirstError(llvm::raw_ostream &OS, DiagnosticOptions &DO) : Printer(new TextDiagnosticPrinter(BufOS, DO)), OS(OS) {
    BufOS.enable_colors(DO.showColors(false)); // -fcolor-diagnostics
  }
  void BeginSourceFile(const LangOptions &LO, const Preprocessor *PP) override { Printer->BeginSourceFile(LO, PP); }
  void EndSourceFile() override { Printer->EndSourceFile(); }
  void clear() override {
    DiagnosticConsumer::clear();
    Printer->clear();
  }
  void HandleDiagnostic(DiagnosticsEngine::Level L, const Diagnostic &Info) override {
    SourceLocation At = Info.getLocation();
    if (IP && At.isValid() && Info.hasSourceManager()) {
      const SourceManager &SM = Info.getSourceManager();
      if (SM.getFileID(SM.getFileLoc(At)) == SM.getMainFileID()) {
        SmallString<100> Msg;
        Info.FormatDiagnostic(Msg);
        DiagnosticStorage None;
        return handle(L, Diagnostic(Info.getDiags(), IP->CellEnd, Info.getID(), None, Msg));
      }
    }
    handle(L, Info);
  }

private:
  void handle(DiagnosticsEngine::Level L, const Diagnostic &Info) {
    DiagnosticConsumer::HandleDiagnostic(L, Info);
    if (L >= DiagnosticsEngine::Error && !Seen) {
      Seen = true;
      Loc = Info.getLocation();
    }
    Printer->HandleDiagnostic(L, Info);
    std::string Included = "In file included from " + std::string(MainFile) + ":1:\n";
    for (size_t P; (P = Buf.find(Included)) != std::string::npos;)
      Buf.erase(P, Included.size());
    OS << Buf;
    Buf.clear();
  }
};

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
  std::map<std::string, llvm::FunctionType *> Types; // the functions among them
  std::set<std::string> Wanted; // internal names used before a cell defines them
  FirstError *First = nullptr;  // the diagnostics' client
  unsigned N = 0;
};

std::unique_ptr<ScSession> ScSession::create(ScOptions O, llvm::raw_ostream *Diags) {
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
  I.First = new FirstError(Diags ? *Diags : llvm::errs(), I.CI->getDiagnosticOpts());
  I.CI->createDiagnostics(I.First);
  // the main file is empty, the cells come after it (-Wpedantic)
  I.CI->getDiagnostics().setSeverity(diag::ext_empty_translation_unit, diag::Severity::Ignored, {});
  I.Ctx.setDiagnosticHandler(std::make_unique<BackendDiags>(I.CI->getDiagnostics()));
  I.Act = std::make_unique<SessionAction>(I.Ctx);
  if (!I.CI->ExecuteAction(*I.Act) || I.CI->getDiagnostics().hasErrorOccurred())
    return nullptr;
  I.IP = std::make_unique<IncrementalParser>(*I.CI, I.Act->CG, I.Ctx);
  I.First->IP = I.IP.get();
  I.IP->OneCell = O.OneCell;
  // a static used in an earlier cell is emitted where it is defined: in one
  // translation unit the use would take it out of the CodeGen's deferred ones
  I.IP->BeforeEnd = [&I] {
    CodeGenerator *CG = I.Act->CG;
    for (const std::string &N : I.Wanted) {
      const Decl *D = CG->GetDeclForMangledName(N);
      if (auto *F = llvm::dyn_cast_or_null<FunctionDecl>(D); F && F->isDefined() && !llvm::isa<CXXConstructorDecl>(F) &&
                                                             !llvm::isa<CXXDestructorDecl>(F))
        CG->GetAddrOfGlobal(GlobalDecl(F), false);
      else if (auto *V = llvm::dyn_cast_or_null<VarDecl>(D); V && V->hasDefinition() == VarDecl::Definition)
        CG->GetAddrOfGlobal(GlobalDecl(V), false);
    }
  };
  I.Initial = I.Act->CG->ReleaseModule();
  I.Act->CG->StartModule("sc_cell_1", I.Ctx);
  std::string Prelude = ShowPrelude;
  if (I.CI->getLangOpts().CPlusPlus)
    Prelude = "extern \"C\" {\n" + Prelude + "}\n";
  if (!I.IP->Parse(Prelude, /*Prelude=*/true))
    return nullptr;
  I.Act->CG->ReleaseModule(); // declarations only
  I.Act->CG->StartModule("sc_cell_1", I.Ctx);
  return S;
}

ScSession::~ScSession() = default;

// No token in Src after its byte Off.
static bool onlySpaceAfter(llvm::StringRef Src, unsigned Off, const LangOptions &LO) {
  std::string Rest = Src.substr(std::min<size_t>(Off, Src.size())).str(); // the lexer reads its NUL
  Lexer L(SourceLocation(), LO, Rest.data(), Rest.data(), Rest.data() + Rest.size());
  Token T;
  L.LexFromRawLexer(T);
  return T.is(tok::eof);
}

ScStatus ScSession::cell(llvm::StringRef Src, bool Backend, std::string &Obj,
                         std::vector<std::string> &Syms, bool Whole) {
  Impl &I = *this->I;
  CompilerInstance &CI = *I.CI;
  Obj.clear();
  Syms.clear();
  I.First->Seen = false;
  if (!I.IP->Parse(Src)) {
    // the parser wanted more where the cell ends: after its last token, or
    // at the end of the input, which FirstError puts there
    SourceManager &SM = CI.getSourceManager();
    if (Whole && I.First->Seen && I.First->Loc.isValid()) {
      auto [FID, Off] = SM.getDecomposedLoc(SM.getFileLoc(I.First->Loc));
      if (FID == I.IP->LastFID && onlySpaceAfter(Src, Off, CI.getLangOpts()))
        return ScIncomplete;
    }
    return ScError;
  }
  CodeGenerator *CG = I.Act->CG;
  std::unique_ptr<llvm::Module> M = CG->ReleaseModule();
  CG->StartModule("sc_cell_" + std::to_string(++I.N + 1), I.Ctx);
  if (!M)
    return ScLost;
  retypeDecls(*M, I.Types);
  if (!I.IP->OneCell) // for the cells after
    scPromoteInternals(*M, I.Known, [&](llvm::StringRef Name) { return CG->GetDeclForMangledName(Name); });
  Syms = scExports(*M);
  I.Known.insert(Syms.begin(), Syms.end());
  for (const std::string &N : Syms)
    if (llvm::Function *F = M->getFunction(N))
      I.Types.emplace(N, F->getFunctionType());
  // an always_inline definition the backend drops (available_externally) is
  // emitted again where a later cell uses it, as in one translation unit
  for (llvm::Function &F : *M)
    if (F.hasAvailableExternallyLinkage())
      if (const Decl *D = CG->GetDeclForMangledName(F.getName()))
        CG->HandleTopLevelDecl(DeclGroupRef(const_cast<Decl *>(D)));
  for (llvm::GlobalValue &GV : M->global_values())
    if (GV.getName().ends_with(".sc")) {
      std::string N = GV.getName().drop_back(3).str();
      if (!GV.isDeclaration())
        I.Wanted.erase(N);
      else if (!I.Known.count(GV.getName().str()))
        I.Wanted.insert(N);
    }
  if (!Backend)
    return ScOk;
  llvm::SmallString<0> Buf;
  scSetLLVMArgs(CI.getInvocation()); // after a link, which resets them
  emitBackendOutput(CI, CI.getCodeGenOpts(), CI.getTarget().getDataLayoutString(), M.get(),
                    Backend_EmitObj, CI.getVirtualFileSystemPtr(),
                    std::make_unique<llvm::raw_svector_ostream>(Buf));
  if (CI.getDiagnostics().hasErrorOccurred())
    return ScLost;
  Obj.assign(Buf.begin(), Buf.end());
  return ScOk;
}

ScStatus ScSession::type(llvm::StringRef Src, std::string &Type) {
  Type.clear();
  I->IP->TypeOf = &Type;
  I->IP->Parse(Src);
  I->IP->TypeOf = nullptr;
  return Type.empty() ? ScError : ScOk;
}

unsigned ScSession::depth(llvm::StringRef Src) {
  std::string Buf = Src.str(); // the lexer reads its NUL
  Lexer L(SourceLocation(), I->CI->getLangOpts(), Buf.data(), Buf.data(), Buf.data() + Buf.size());
  L.SetKeepWhitespaceMode(true); // an unterminated comment is a token
  int D = 0;
  bool Open = false;
  Token T;
  do {
    L.LexFromRawLexer(T);
    switch (T.getKind()) {
    case tok::l_paren: case tok::l_square: case tok::l_brace: ++D; break;
    case tok::r_paren: case tok::r_square: case tok::r_brace: D = std::max(D - 1, 0); break;
    case tok::unknown: {
      llvm::StringRef Text(L.getBufferLocation() - T.getLength(), T.getLength());
      Open = Text.starts_with("/*") && (Text.size() < 4 || !Text.ends_with("*/"));
      break;
    }
    case tok::eof: break;
    default: Open = false;
    }
  } while (T.isNot(tok::eof));
  Open = Open || Src.rtrim(" \t\r").ends_with("\\");
  return D + Open;
}
