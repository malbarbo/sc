#!/usr/bin/env python3
"""Functional categories for funcs.tsv (attrib.py output). usage: cat2.py funcs.tsv [-v]"""
import csv, collections, re, sys
CATS = [
 ('C++/ObjC/OpenMP/GPU/outros alvos (resíduo)', r'clangSema/(SemaOpenMP|SemaTemplate\w*|SemaOverload|SemaDeclCXX|SemaSYCL|SemaCodeComplete|SemaExprCXX|SemaExprObjC|SemaDeclObjC|SemaObjC\w*|SemaHLSL|SemaOpenACC\w*|SemaAMDGPU|SemaARM|SemaRISCV|SemaX86|SemaTypeTraits|SemaLambda|SemaConcept|SemaCoroutine|SemaCUDA|SemaAccess|SemaCXXScopeSpec|SemaExceptionSpec|SemaModule|SemaPseudoObject|SemaAPINotes|CodeCompleteConsumer|SemaOpenCL|SemaSwift|Sema(PPC|MIPS|SystemZ|NVPTX|Hexagon|LoongArch|BPF|AVR|M68k|MSP430|SPIRV|DirectX))|clangParse/(ParseDeclCXX|ParseObjc|ParseExprCXX|ParseTemplate|ParseCXXInlineMethods|ParseTentative|ParseOpenMP|ParseOpenACC|ParseHLSL\w*)|clangAST/(ItaniumMangle|VTableBuilder|DeclCXX|ExprCXX|DeclTemplate|DeclObjC|ExprObjC|StmtObjC|ASTStructuralEquivalence|ParentMapContext|DynamicRecursiveASTVisitor|CXXInheritance|TemplateBase|TemplateName|ODRHash|MicrosoftMangle|OpenMPClause|StmtOpenMP|ASTImporter\w*)|clangCodeGen/(ItaniumCXXABI|CGVTables|CGCXX\w*|CGClass|CGObjC\w*|CGOpenMP\w*|CGBlocks|CGException|CGDeclCXX|CGExprCXX|CGVTT|ObjectFilePCHContainerWriter|MicrosoftCXXABI|CGCoroutine|CGHLSL\w*|CGCUDA\w*)|ProfileData/ItaniumManglingCanonicalizer|Demangle/'),
 ('Opções/invocação (CompilerInvocation, cl::opt)', r'clangFrontend/CompilerInvocation|clangOptions|Option/|Support/CommandLine'),
 ('Debug info (clang+LLVM)', r'CGDebugInfo|AsmPrinter/(Dwarf\w*|DIE\w*|CodeView\w*|DebugHandlerBase|AccelTable|AddressPool)|DebugInfoCodeView|DebugInfoMetadata|InstrRefBasedImpl|VarLocBasedImpl|LiveDebug\w*|AssignmentTracking\w*|MCDwarf|BinaryFormat/Dwarf|DebugProgramInstruction|Debugify'),
 ('IR textual/bitcode/upgrade/verifier', r'LLVMAsmParser|BitReader|BitWriter|AutoUpgrade|Core/AsmWriter|Core/Verifier|MachineVerifier|MIRPrinter|MetadataLoader|ValueEnumerator'),
 ('PGO/profile/coverage', r'ProfileData|CodeGenPGO|CoverageMapping|CGData|SampleProf|MIRSampleProfile'),
 ('Passes LLVM de otimização (IR e Machine) e PassBuilder', r'Passes/|ScalarEvolution\w*|DependenceAnalysis|ModuleSummaryAnalysis|StackSafetyAnalysis|IRSimilarity|MemorySSA\w*|LazyValueInfo|LoopAccessAnalysis|InlineCost|TransformUtils/(InlineFunction|SimplifyCFG|ScalarEvolutionExpander|CodeLayout|Local)|CodeGen/(CodeGenPrepare|MachinePipeliner|ModuloSchedule|MachineScheduler|RegAllocGreedy|MachineSink|MachineBlockPlacement|RegisterCoalescer|MachineOutliner|IfConversion|SplitKit|InlineSpiller|SelectOptimize|GlobalMergeFunctions|BranchFolding|MachineLICM|MachineCopyPropagation|ComplexDeinterleavingPass|EarlyIfConversion|MachineCSE|TailDuplicat\w*|LiveIntervals|MachineCombiner|RegAllocBasic|StackColoring|ScheduleDAGInstrs|FixupStatepointCallerSaved)|Coroutines|Linker|ipo/|Vectorize|InstCombine|Scalar/|GlobalISel|WebAssemblyPreLegalizerCombiner|WebAssembly(InstructionSelector|CallLowering|LegalizerInfo|RegisterBankInfo|PostLegalizer\w*)'),
 ('Formatos objeto não-wasm (ELF/MachO/COFF/XCOFF/TextAPI)', r'Object/(ELF\w*|MachO\w*|COFF\w*|XCOFF\w*|GOFF\w*|Minidump|Offloading|OffloadBinary|DXContainer)|TextAPI|MC/(ELF\w*|MachO\w*|MCMachO\w*|MCELF\w*|WinCOFF\w*|MCWinCOFF\w*|XCOFF\w*|MCXCOFF\w*|GOFF\w*|MCGOFF\w*|MCCOFF\w*|SPIRV\w*|DXContainer\w*|MCDXContainer\w*|MCWin64EH|MCWinEH)|MCParser/(Darwin|ELF|COFF\w*|WinCOFF|GOFF|XCOFF)AsmParser|TargetParser/(AArch64|RISCV|X86|ARM|LoongArch|CSKY|PPC|AMDGPU)\w*'),
 ('Inicializadores estáticos (crt1)', r'crt1-command'),
 ('libc/libc++', r'^c\+\+/|^c/'),
 ('lld (wasm)', r'^lld'),
 ('Frontend clang (resto)', r'clangFrontend|clangRewrite|cc1_main|sc_main'),
 ('Frontend C: Lex/Parse/Basic', r'clangLex|clangParse|clangBasic'),
 ('Frontend C: Sema', r'clangSema|clangAnalysis'),
 ('Frontend C: AST', r'clangAST'),
 ('CodeGen clang (C → IR)', r'clangCodeGen'),
 ('LLVM SelectionDAG', r'SelectionDAG'),
 ('LLVM CodeGen/AsmPrinter/MC (O0)', r'LLVMCodeGen|AsmPrinter|LLVMMC|MCParser'),
 ('Alvo WebAssembly', r'WebAssembly'),
 ('LLVM IR/Analysis/Support/Transform', r'LLVMCore|LLVMAnalysis|LLVMSupport|TransformUtils|TargetParser|BinaryFormat|LLVMObject|LLVMTarget|Demangle'),
 ('?', r'.'),
]
rows = [(int(r[0]), r[1][3:-2] if r[1].startswith('lib') else r[1].lstrip('<').rstrip('>'), r[2]) for r in csv.reader(open(sys.argv[1]), delimiter='\t')]
T = sum(r[0] for r in rows)
C = collections.Counter(); ex = collections.defaultdict(collections.Counter)
for s, l, o in rows:
    k = f'{l.replace("LLVM","LLVM").replace("clang","clang")}/{o}'
    for name, rx in CATS:
        if re.search(rx, k):
            C[name] += s; ex[name][k] += s; break
for name, _ in CATS:
    v = C[name]
    print(f'{v/1e6:6.2f} MB {100*v/T:5.1f}%  {name}')
    if '-v' in sys.argv:
        print('        ' + ', '.join(f'{k.split("/")[-1]} {v//1000}K' for k, v in ex[name].most_common(12)))
print(f'{T/1e6:6.2f} MB total código')
