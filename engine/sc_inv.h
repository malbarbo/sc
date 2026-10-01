#pragma once
#include "llvm/ADT/ArrayRef.h"
#include <string>
#include <vector>

namespace clang { class CompilerInvocation; }

struct ScOptions {
  std::string Input, Output = "-", Sysroot, ResourceDir;
  std::vector<std::string> ISystem, ExternCISystem, Include, Warnings;
  std::vector<std::pair<std::string, std::string>> ModuleFiles; // -fmodule-file=<name>=<pcm>
  bool CXX = false;
  bool PIC = false;            // -mrelocation-model pic -pic-level 2 (a shared object)
  bool VisibilityDefault = false; // -fvisibility=default
  bool ModuleInterface = false; // -emit-module-interface -fmodules-reduced-bmi (a .pcm)
};

bool scParseArgs(llvm::ArrayRef<const char *> Args, ScOptions &O);
void scBuildInvocation(clang::CompilerInvocation &Inv, const ScOptions &O);
