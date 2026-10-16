#pragma once
#include "llvm/ADT/ArrayRef.h"
#include <string>
#include <vector>

namespace clang { class CompilerInvocation; }

struct ScOptions {
  std::string Input, Output = "-", Sysroot, ResourceDir;
  std::string Std;             // -std=, for the corpus only (default gnu23, gnu++26)
  std::vector<std::string> ISystem, ExternCISystem, Include, Warnings;
  std::vector<std::string> Macros; // -D
  // -fsanitize=, -fsanitize-trap=, -fsanitize-merge=: the checks' names
  std::vector<std::string> Sanitize, SanitizeTrap, SanitizeMerge;
  std::vector<std::pair<std::string, std::string>> ModuleFiles; // -fmodule-file=<name>=<pcm>
  bool CXX = false;
  bool PIC = false;            // -mrelocation-model pic -pic-level 2 (a shared object)
  bool VisibilityDefault = false; // -fvisibility=default
  bool ModuleInterface = false; // -emit-module-interface -fmodules-reduced-bmi (a .pcm)
  bool Colors = false;         // -fcolor-diagnostics
  bool OneCell = false;        // -sc-one-cell: a session of one cell, the whole program
  bool LineTables = false;     // -debug-info-kind=line-tables-only: where a trap is
  bool PatternInit = false;    // -ftrivial-auto-var-init=pattern
  // -fno-sanitize-memory-param-retval, -fno-sanitize-address-use-odr-indicator:
  // what the driver adds with any sanitizer
  bool NoParamRetval = false, NoOdrIndicator = false;
};

// false, with the reason in Error, for a flag the engine does not take
bool scParseArgs(llvm::ArrayRef<const char *> Args, ScOptions &O, std::string &Error);
void scBuildInvocation(clang::CompilerInvocation &Inv, const ScOptions &O);
// sets the backend options the invocation lists (FrontendOptions::LLVMArgs, the
// driver's -mllvm), as cc1 does, but without the command line parser. Those
// already given are left alone: a session sets them again before each
// backend, since wasm-ld resets every option (cl::ResetAllOptionOccurrences)
void scSetLLVMArgs(const clang::CompilerInvocation &Inv);
